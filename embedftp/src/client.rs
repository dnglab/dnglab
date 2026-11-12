// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use chrono::{Datelike, Timelike, Utc};
use futures::TryStreamExt;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use glob::glob;
use log::{debug, error, info, warn};
use std::ffi::OsString;
use std::io;
use std::mem;
use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf, StripPrefixError};
use std::result;
use std::sync::Arc;
use tokio::fs::{File, canonicalize, create_dir, metadata, read_dir, remove_dir_all, remove_file};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio_util::codec::{Decoder, Framed};

#[cfg(unix)]
use std::os::unix::prelude::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use crate::codec::FtpCodec;
use crate::command::{Command, ParseError, TransferType};
use crate::config::{Config, FtpCallback};
use crate::error::{Error, Result};
use crate::ftp::{Answer, ResultCode};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

type Writer = SplitSink<Framed<TcpStream, FtpCodec>, Answer>;

/// How the data connection of the next transfer is opened
enum DataMode {
  /// Neither PORT nor PASV or EPSV was received
  None,
  /// Active mode: connect to this port of the client
  Active(u16),
  /// Passive mode: accept the connection of the client, for a single transfer
  Passive(TcpListener),
}

/// Client handler
struct Client<T>
where
  T: FtpCallback + Clone + Send,
{
  cwd: PathBuf,
  data_mode: DataMode,
  /// EPSV ALL was received, the client uses no other command to set up data connections.
  epsv_all: bool,
  data_reader: Option<OwnedReadHalf>,
  data_writer: Option<OwnedWriteHalf>,
  #[allow(dead_code)]
  handle: Handle,
  name: Option<String>,
  server_root: PathBuf,
  transfer_type: TransferType,
  writer: Writer,
  config: Config,
  waiting_password: bool,
  local_addr: SocketAddr,
  remote_addr: SocketAddr,
  env: T,
}

impl<T> Client<T>
where
  T: FtpCallback + Clone + Send,
{
  fn new(handle: Handle, writer: Writer, server_root: PathBuf, config: Config, env: T, local_addr: SocketAddr, remote_addr: SocketAddr) -> Self {
    Client {
      cwd: PathBuf::from("/"),
      data_mode: DataMode::None,
      epsv_all: false,
      data_reader: None,
      data_writer: None,
      handle,
      name: None,
      server_root,
      transfer_type: TransferType::Ascii,
      writer,
      config,
      waiting_password: false,
      local_addr,
      remote_addr,
      env,
    }
  }

  /// Check if current client is logged in.
  fn is_logged(&self) -> bool {
    self.name.is_some() && !self.waiting_password
  }

  /// Handle a new COMMAND from the client
  /// Each handler tries to handle errors and return a possible
  /// error back to client. Only for hard errors like broken
  /// connections, this function returns an error to the
  /// loop. This would terminate the connection.
  async fn handle_cmd(&mut self, cmd: Command) -> Result<()> {
    debug!("Received command: {:?}", cmd);
    if self.is_logged() {
      match cmd {
        Command::Cwd(directory) => return self.cwd(directory).await,
        Command::List(path) => return self.list(path).await,
        Command::Nlst(path) => return self.nlst(path).await,
        Command::Pasv | Command::Port(_) if self.epsv_all => {
          return self
            .send(Answer::new(ResultCode::InvalidParameterOrArgument, "Only EPSV is allowed after EPSV ALL"))
            .await;
        }
        Command::Pasv => return self.pasv().await,
        Command::Epsv(proto) => return self.epsv(proto).await,
        Command::Port(port) => {
          self.data_mode = DataMode::Active(port);
          return self.send(Answer::new(ResultCode::Ok, &format!("Data port is now {}", port))).await;
        }
        Command::Pwd => {
          let message = format!("{} is the current directory", quote_path(&self.cwd));
          return self.send(Answer::new(ResultCode::PATHNAMECreated, &message)).await;
        }
        Command::Retr(file) => return self.retr(file).await,
        Command::Stor(file) => return self.stor(file).await,
        Command::CdUp => {
          debug!("old Path: {:?}", self.cwd);
          if let Some(path) = self.cwd.parent().map(Path::to_path_buf) {
            self.cwd = path;
            prefix_slash(&mut self.cwd);
          }
          debug!("New Path: {:?}", self.cwd);
          let msg = format!("\"{}\" is the current directory.", self.cwd.to_str().unwrap_or(""));
          return self.send(Answer::new(ResultCode::RequestedFileActionOkay, &msg)).await;
        }
        Command::Mkd(path) => return self.mkd(path).await,
        Command::Rmd(path) => return self.rmd(path).await,
        Command::Dele(path) => return self.dele(path).await,
        _ => (),
      }
    } else if self.name.is_some() && self.waiting_password {
      if let Command::Pass(content) = cmd {
        let mut ok = false;
        if self.config.anonymous {
          ok = true;
        } else {
          for user in &self.config.users {
            if Some(&user.name) == self.name.as_ref() && user.password == content {
              ok = true;
              break;
            }
          }
        }
        if ok {
          self.waiting_password = false;
          let name = self.name.clone().unwrap_or_default();
          self.send(Answer::new(ResultCode::UserLoggedIn, &format!("Welcome {}", name))).await?;
        } else {
          self.send(Answer::new(ResultCode::NotLoggedIn, "Invalid password")).await?;
        }
        return Ok(());
      }
    }
    match cmd {
      Command::Auth => self.send(Answer::new(ResultCode::CommandNotImplemented, "Not implemented")).await?,
      Command::Quit => self.quit().await?,
      Command::Syst => {
        self.send(Answer::new(ResultCode::SystemType, "UNIX Type: L8")).await?;
      }
      Command::Type(typ) => {
        self.transfer_type = typ;
        self.send(Answer::new(ResultCode::Ok, "Transfer type changed successfully")).await?;
      }
      Command::User(content) => {
        if content.is_empty() {
          self.send(Answer::new(ResultCode::InvalidParameterOrArgument, "Invalid username")).await?;
        } else {
          let mut name = None;
          let mut pass_required = true;

          if self.config.anonymous && content == "anonymous" {
            name = Some(content.clone());
          }

          if name.is_none() {
            for user in &self.config.users {
              if user.name == content {
                name = Some(content.clone());
                pass_required = !user.password.is_empty();
                break;
              }
            }
          }
          if name.is_none() {
            self.send(Answer::new(ResultCode::NotLoggedIn, "Unknown user...")).await?;
          } else {
            self.name = name.clone();
            if pass_required {
              self.waiting_password = true;
              self
                .send(Answer::new(
                  ResultCode::UserNameOkayNeedPassword,
                  &format!(
                    "Login OK, password needed for {}",
                    name.as_deref().expect("name verified as Some in else branch")
                  ),
                ))
                .await?;
            } else {
              self.waiting_password = false;
              self.send(Answer::new(ResultCode::UserLoggedIn, &format!("Welcome {}!", content))).await?;
            }
          }
        }
      }
      Command::Feat => {
        let features = [String::from("UTF8")];
        self.send(Answer::new_multiline(ResultCode::SystemStatus, "Feature list", &features)).await?;
      }
      Command::NoOp => self.send(Answer::new(ResultCode::Ok, "Doing nothing")).await?,
      Command::Unknown(s) => {
        warn!("Unknown command");
        self
          .send(Answer::new(ResultCode::UnknownCommand, &format!("\"{}\": Not implemented", s)))
          .await?
      }
      _ => {
        warn!("Please login first");
        // It means that the user tried to send a command while they weren't logged yet.
        self.send(Answer::new(ResultCode::NotLoggedIn, "Please log first")).await?;
      }
    }
    Ok(())
  }

  /// Answer a command line that is no valid command
  async fn reject(&mut self, err: ParseError) -> Result<()> {
    warn!("Rejected command: {:?}", err);
    let answer = match err {
      ParseError::Syntax(msg) => Answer::new(ResultCode::InvalidParameterOrArgument, msg),
      ParseError::Parameter(msg) => Answer::new(ResultCode::CommandNotImplementedForThatParameter, msg),
    };
    self.send(answer).await
  }

  /// Open the data connection of a transfer: connect to the client in active mode,
  /// accept the connection of the client in passive mode.
  /// Returns false if there is no data connection.
  async fn initiate_data_connection(&mut self) -> bool {
    self.close_data_connection();
    let stream = match mem::replace(&mut self.data_mode, DataMode::None) {
      DataMode::None => return false,
      DataMode::Active(port) => {
        // The port is used for further transfers, too
        self.data_mode = DataMode::Active(port);
        let mut addr = self.remote_addr;
        addr.set_port(port);
        TcpStream::connect(addr).await
      }
      DataMode::Passive(listener) => listener.accept().await.map(|(stream, _addr)| stream),
    };
    match stream {
      Ok(stream) => {
        let (r, w) = stream.into_split();
        self.data_writer = Some(w);
        self.data_reader = Some(r);
        true
      }
      Err(e) => {
        error!("Failed to open data connection: {}", e);
        false
      }
    }
  }

  /// Close the data connection and send the final reply of a transfer command
  async fn finish_transfer(&mut self, answer: Answer) -> Result<()> {
    self.close_data_connection();
    self.send(answer).await
  }

  fn close_data_connection(&mut self) {
    self.data_reader = None;
    self.data_writer = None;
  }

  async fn complete_path(&self, path: &Path) -> result::Result<PathBuf, io::Error> {
    let directory = self.server_root.join(if path.has_root() {
      path.iter().skip(1).clone().collect()
    } else {
      path.to_path_buf()
    });
    let dir = canonicalize(&directory).await;
    if let Ok(ref dir) = dir {
      if !dir.starts_with(&self.server_root) {
        return Err(io::ErrorKind::PermissionDenied.into());
      }
    }
    dir
  }

  async fn mkd(&mut self, path: PathBuf) -> Result<()> {
    let fullpath = self.cwd.join(&path);
    let parent = get_parent(fullpath.clone());
    if let Some(parent) = parent {
      let parent = parent.to_path_buf();
      let res = self.complete_path(&parent).await;
      if let Ok(mut dir) = res {
        let is_dir = metadata(&dir).await.map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
          let filename = get_filename(fullpath);
          if let Some(filename) = filename {
            dir.push(filename);
            if create_dir(&dir).await.is_ok() {
              let mut created = self.strip_prefix(dir).unwrap_or_default();
              prefix_slash(&mut created);
              let message = format!("{} created", quote_path(&created));
              self.send(Answer::new(ResultCode::PATHNAMECreated, &message)).await?;
              return Ok(());
            }
          }
        }
      }
    }
    let msg = format!("Couldn't create folder: {}", path.to_string_lossy());
    warn!("{}", msg);
    self.send(Answer::new(ResultCode::FileNotFound, &msg)).await?;
    Ok(())
  }

  async fn rmd(&mut self, directory: PathBuf) -> Result<()> {
    let path = self.cwd.join(&directory);
    let res = self.complete_path(&path).await;
    if let Ok(dir) = res {
      if remove_dir_all(dir).await.is_ok() {
        self
          .send(Answer::new(ResultCode::RequestedFileActionOkay, "Folder successfully removed"))
          .await?;
        return Ok(());
      }
    }
    let msg = format!("Couldn't remove folder: {}", directory.to_string_lossy());
    warn!("{}", msg);
    self.send(Answer::new(ResultCode::FileNotFound, &msg)).await?;
    Ok(())
  }

  async fn dele(&mut self, file: PathBuf) -> Result<()> {
    let path = self.cwd.join(&file);
    let res = self.complete_path(&path).await;
    if let Ok(f) = res {
      if remove_file(f).await.is_ok() {
        self.send(Answer::new(ResultCode::RequestedFileActionOkay, "File successfully removed")).await?;
        return Ok(());
      }
    }
    let msg = format!("Couldn't remove file: {}", file.to_string_lossy());
    warn!("{}", msg);
    self.send(Answer::new(ResultCode::FileNotFound, &msg)).await?;
    Ok(())
  }

  fn strip_prefix(&self, dir: PathBuf) -> result::Result<PathBuf, StripPrefixError> {
    let res = dir.strip_prefix(&self.server_root).map(|p| p.to_path_buf());
    res
  }

  async fn cwd(&mut self, directory: PathBuf) -> Result<()> {
    let path = self.cwd.join(&directory);
    let res = self.complete_path(&path).await;
    if let Ok(dir) = res {
      let is_dir = metadata(&dir).await.map(|m| m.is_dir()).unwrap_or(false);
      let res = self.strip_prefix(dir);
      if let (true, Ok(prefix)) = (is_dir, res) {
        self.cwd = prefix.to_path_buf();
        prefix_slash(&mut self.cwd);
        self
          .send(Answer::new(
            ResultCode::RequestedFileActionOkay,
            &format!("Directory changed to \"{}\"", directory.display()),
          ))
          .await?;
        return Ok(());
      }
    }
    warn!("No such file or directory");
    self.send(Answer::new(ResultCode::FileNotFound, "No such file or directory")).await?;
    Ok(())
  }

  async fn list(&mut self, path: Option<PathBuf>) -> Result<()> {
    let answer = self.transfer_list(path).await?;
    self.finish_transfer(answer).await
  }

  /// Send the listing for LIST, returns the final reply
  async fn transfer_list(&mut self, path: Option<PathBuf>) -> Result<Answer> {
    if !self.initiate_data_connection().await {
      warn!("No opened data connection");
      return Ok(no_data_connection());
    }
    let path = self.cwd.join(path.unwrap_or_default());
    let Ok(path) = self.complete_path(&path).await else {
      warn!("No such file or directory");
      return Ok(Answer::new(ResultCode::FileNotFound, "No such file or directory"));
    };
    let mut out = vec![];
    let is_dir = metadata(&path).await.map(|m| m.is_dir()).unwrap_or(false);
    if is_dir {
      match read_dir(&path).await {
        Ok(mut dir) => {
          while let Ok(Some(entry)) = dir.next_entry().await {
            add_file_info(entry.path(), &mut out).await;
          }
        }
        Err(_) => return Ok(Answer::new(ResultCode::FileNotFound, "No such file or directory")),
      }
    } else {
      add_file_info(path, &mut out).await;
    }
    self
      .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to list directory..."))
      .await?;
    Ok(self.send_data(out).await)
  }

  async fn nlst(&mut self, path: Option<PathBuf>) -> Result<()> {
    let answer = self.transfer_nlst(path).await?;
    self.finish_transfer(answer).await
  }

  /// Send the listing for NLST, returns the final reply
  async fn transfer_nlst(&mut self, path: Option<PathBuf>) -> Result<Answer> {
    if !self.initiate_data_connection().await {
      warn!("No opened data connection");
      return Ok(no_data_connection());
    }
    let cwd = self.cwd.clone();
    let Ok(cwd) = self.complete_path(&cwd).await else {
      return Ok(Answer::new(ResultCode::FileNotFound, "No such file or directory"));
    };
    let mut out = vec![];

    let pattern = if let Some(p) = path {
      if p.is_absolute() {
        return Ok(Answer::new(ResultCode::InvalidParameterOrArgument, "No such file or directory"));
      }
      cwd.join(p)
    } else {
      cwd.join("*")
    };

    let matches = match pattern.to_str().map(|s| glob(s).map_err(|_| Error::from("Invalid glob pattern"))) {
      Some(Ok(matches)) => matches,
      _ => return Ok(Answer::new(ResultCode::InvalidParameterOrArgument, "Invalid pattern specified")),
    };

    for entry in matches {
      match entry {
        Ok(path) => match canonicalize(&path).await {
          Ok(p) => {
            if p.starts_with(&self.server_root) {
              add_file_info_nlst(path, &mut out).await;
            } else {
              warn!("Entry is out of server root: {:?}, skipping", path);
            }
          }
          Err(e) => {
            error!("Error while processing NLST command: {:?}", e);
          }
        },
        Err(e) => {
          error!("Error while processing NLST command: {:?}", e);
        }
      }
    }

    self
      .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to list directory..."))
      .await?;
    Ok(self.send_data(out).await)
  }

  /// Listen on a free port for the data connection of the next transfer
  async fn listen_passive(&mut self) -> io::Result<u16> {
    self.close_data_connection();
    self.data_mode = DataMode::None;
    let mut addr = self.local_addr;
    addr.set_port(0); // auto configure port
    let listener = TcpListener::bind(&addr).await?;
    let port = listener.local_addr()?.port();
    debug!("Waiting for data clients on port {}...", port);
    self.data_mode = DataMode::Passive(listener);
    Ok(port)
  }

  async fn pasv(&mut self) -> Result<()> {
    // If it's a IPv4 address but mapped into v6 as defined in IETF RFC 4291 section 2.5.5.2,
    // use the canonical value.
    let IpAddr::V4(v4addr) = self.local_addr.ip().to_canonical() else {
      return self
        .send(Answer::new(
          ResultCode::CantOpenDataConnection,
          "PASV is not possible for IPv6 connections, use EPSV",
        ))
        .await;
    };
    let answer = match self.listen_passive().await {
      Ok(port) => {
        let octets = v4addr.octets();
        Answer::new(
          ResultCode::EnteringPassiveMode,
          &format!(
            "Entering passive mode ({},{},{},{},{},{}).",
            octets[0],
            octets[1],
            octets[2],
            octets[3],
            port >> 8,
            port & 0xFF
          ),
        )
      }
      Err(e) => passive_failed(e),
    };
    self.send(answer).await
  }

  async fn epsv(&mut self, proto: Option<String>) -> Result<()> {
    if let Some(proto) = proto {
      if proto.to_uppercase().eq("ALL") {
        self.epsv_all = true;
        return self.send(Answer::new(ResultCode::Ok, "Only EPSV is allowed from now on")).await;
      }
    }
    let answer = match self.listen_passive().await {
      Ok(port) => Answer::new(ResultCode::ExtendedEnteringPassiveMode, &format!(" (|||{}|)", port,)),
      Err(e) => passive_failed(e),
    };
    self.send(answer).await
  }

  async fn quit(&mut self) -> Result<()> {
    if self.data_writer.is_some() {
      self.close_data_connection();
    }
    self
      .send(Answer::new(ResultCode::ServiceClosingControlConnection, "Closing connection..."))
      .await?;
    self.writer.close().await?;
    Ok(())
  }

  async fn retr(&mut self, path: PathBuf) -> Result<()> {
    let answer = self.transfer_retr(path).await?;
    self.finish_transfer(answer).await
  }

  /// Send the file for RETR, returns the final reply
  async fn transfer_retr(&mut self, path: PathBuf) -> Result<Answer> {
    if !self.initiate_data_connection().await {
      warn!("No open data connection");
      return Ok(no_data_connection());
    }
    let path = self.cwd.join(path);
    let not_found = Answer::new(ResultCode::FileNotFound, &format!("\"{}\" doesn't exist", path.display()));
    let Ok(path) = self.complete_path(&path).await else {
      warn!("No path");
      return Ok(not_found);
    };
    let is_file = metadata(&path).await.map(|m| m.is_file()).unwrap_or(false);
    if !is_file {
      warn!("No path, not exist");
      return Ok(not_found);
    }
    let mut file = match File::open(&path).await {
      Ok(file) => file,
      Err(e) => {
        error!("Failed to open file {:?}, {}", path, e);
        return Ok(not_found);
      }
    };
    self
      .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to send file..."))
      .await?;
    let res = match &mut self.data_writer {
      Some(writer) => tokio::io::copy(&mut file, writer).await,
      None => Err(io::ErrorKind::NotConnected.into()),
    };
    Ok(res.map_or_else(transfer_aborted, |_| transfer_done()))
  }

  async fn stor(&mut self, path: PathBuf) -> Result<()> {
    let answer = self.transfer_stor(path).await?;
    self.finish_transfer(answer).await
  }

  /// Receive and store the file for STOR, returns the final reply
  async fn transfer_stor(&mut self, path: PathBuf) -> Result<Answer> {
    if !self.initiate_data_connection().await {
      warn!("No opened data connection");
      return Ok(no_data_connection());
    }
    if invalid_path(&path) {
      warn!("Invalid path for STOR: {:?}", path);
      return Ok(Answer::new(ResultCode::FileNameNotAllowed, "File name not allowed"));
    }
    let path = self.cwd.join(path);
    self
      .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to send file..."))
      .await?;
    let data = match self.receive_data().await {
      Ok(data) => data,
      Err(e) => return Ok(transfer_aborted(e)),
    };
    info!("Received file: {:?}, {} bytes", path, data.len());
    self.close_data_connection();
    match self.put_file(path, Arc::new(data)).await {
      Ok(_) => Ok(transfer_done()),
      Err(e) => {
        error!("Failed STOR command: {:?}", e);
        Ok(Answer::new(ResultCode::LocalErrorInProcessing, "Failed to store the file"))
      }
    }
  }

  /// Put file directly or delegate to a filter
  async fn put_file(&mut self, path: PathBuf, content: Arc<Vec<u8>>) -> Result<()> {
    let path = PathBuf::from(&self.server_root).join(path.iter().skip(1).collect::<PathBuf>());
    let handled = self.env.stor_file(&path, content.clone()).await?;
    if !handled {
      tokio::fs::write(&path, content.as_ref()).await?;
    }
    Ok(())
  }

  /// Receive data from a data connection and take out the reader
  async fn receive_data(&mut self) -> io::Result<Vec<u8>> {
    let mut reader = self.data_reader.take().ok_or(io::ErrorKind::NotConnected)?;
    let mut buf = Vec::new();
    tokio::io::copy(&mut reader, &mut buf).await?;
    Ok(buf)
  }

  /// Send a control answer to the client
  async fn send(&mut self, answer: Answer) -> Result<()> {
    self.writer.send(answer).await?;
    Ok(())
  }

  /// Send bytes to the client, returns the final reply of the transfer
  async fn send_data(&mut self, data: Vec<u8>) -> Answer {
    let res = match &mut self.data_writer {
      Some(writer) => writer.write_all(&data).await,
      None => Err(io::ErrorKind::NotConnected.into()),
    };
    res.map_or_else(transfer_aborted, |_| transfer_done())
  }
}

/// Processing loop for a single active client connection
pub(crate) async fn client<T>(stream: TcpStream, handle: Handle, server_root: PathBuf, config: Config, env: T) -> Result<()>
where
  T: FtpCallback + Clone + Send,
{
  let local_addr = stream.local_addr()?;
  let remote_addr = stream.peer_addr()?;
  let (mut writer, mut reader) = FtpCodec {}.framed(stream).split();
  writer.send(Answer::new(ResultCode::ServiceReadyForNewUser, &config.greeting)).await?;

  let mut client = Client::new(handle, writer, server_root, config, env, local_addr, remote_addr);
  while let Some(cmd) = reader.try_next().await? {
    let quit = cmd == Ok(Command::Quit);
    match cmd {
      Ok(cmd) => client.handle_cmd(cmd).await?,
      Err(err) => client.reject(err).await?,
    }
    if quit {
      break;
    }
  }

  debug!("Client {} closed connection", remote_addr);
  Ok(())
}

/// Final reply of a completed transfer
fn transfer_done() -> Answer {
  Answer::new(ResultCode::ClosingDataConnection, "Transfer done")
}

/// Final reply of a transfer that failed on the data connection
fn transfer_aborted(e: io::Error) -> Answer {
  error!("Data connection failed: {}", e);
  Answer::new(ResultCode::ConnectionClosed, "Data connection failed, transfer aborted")
}

/// Reply of PASV and EPSV without a port to listen on
fn passive_failed(e: io::Error) -> Answer {
  error!("Failed to listen for a data connection: {}", e);
  Answer::new(ResultCode::CantOpenDataConnection, "Unable to listen for a data connection")
}

/// Final reply of a transfer command without a data connection
fn no_data_connection() -> Answer {
  Answer::new(ResultCode::CantOpenDataConnection, "No opened data connection")
}

/// Check if the path contains ".."
fn invalid_path(path: &Path) -> bool {
  for component in path.components() {
    if let Component::ParentDir = component {
      return true;
    }
  }
  false
}

fn get_parent(path: PathBuf) -> Option<PathBuf> {
  path.parent().map(|p| p.to_path_buf())
}

fn get_filename(path: PathBuf) -> Option<OsString> {
  path.file_name().map(|p| p.to_os_string())
}

/// Path name for a 257 reply: in double quotes, with the quotes of the name doubled
fn quote_path(path: &Path) -> String {
  format!("\"{}\"", path.to_string_lossy().replace('"', "\"\""))
}

fn prefix_slash(path: &mut PathBuf) {
  if !path.is_absolute() {
    *path = Path::new("/").join(&path);
  }
}

// If an error occurs when we try to get file's information, we just return and don't send its info.
async fn add_file_info(path: PathBuf, out: &mut Vec<u8>) {
  let meta = match metadata(&path).await {
    Ok(meta) => meta,
    _ => return,
  };
  let extra = if meta.is_dir() { "/" } else { "" };
  let is_dir = if meta.is_dir() { "d" } else { "-" };
  let time: chrono::DateTime<Utc> = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH).into();
  #[cfg(unix)]
  let file_size = meta.size();
  #[cfg(windows)]
  let file_size = meta.file_size();
  let path = match path.to_str() {
    Some(path) => match path.split('/').next_back() {
      Some(path) => path,
      _ => return,
    },
    _ => return,
  };
  // TODO: maybe improve how we get rights in here?
  let rights = if meta.permissions().readonly() { "r--r--r--" } else { "rw-rw-rw-" };

  let file_str = format!(
    "{is_dir}{rights} {links} {owner} {group} {size} {month} {day} {hour}:{min} {path}{extra}\r\n",
    is_dir = is_dir,
    rights = rights,
    links = 1,           // number of links
    owner = "anonymous", // owner name
    group = "anonymous", // group name
    size = file_size,
    month = MONTHS[time.month0() as usize],
    day = time.day(),
    hour = time.hour(),
    min = time.minute(),
    path = path,
    extra = extra
  );
  out.extend(file_str.as_bytes());
  //debug!("==> {:?}", &file_str);
}

// If an error occurs when we try to get file's information, we just return and don't send its info.
async fn add_file_info_nlst(path: PathBuf, out: &mut Vec<u8>) {
  let extra = match metadata(&path).await {
    Ok(meta) if meta.is_dir() => "/",
    _ => "",
  };
  let path = match path.to_str() {
    Some(path) => match path.split('/').next_back() {
      Some(path) => path,
      _ => return,
    },
    _ => return,
  };

  let file_str = format!("{path}{extra}\r\n", path = path, extra = extra);
  out.extend(file_str.as_bytes());
  debug!("==> {:?}", &file_str);
}
