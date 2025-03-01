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
use std::fs::{File, create_dir, read_dir, remove_dir_all, remove_file};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf, StripPrefixError};
use std::result;
use std::sync::Arc;
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
use crate::command::{Command, TransferType};
use crate::config::{Config, FtpCallback};
use crate::error::{Error, Result};
use crate::ftp::{Answer, ResultCode};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

type Writer = SplitSink<Framed<TcpStream, FtpCodec>, Answer>;

/// Client handler
struct Client<T>
where
  T: FtpCallback + Clone + Send,
{
  cwd: PathBuf,
  /// Port to use for the next active data transmission.
  active_data_port: Option<u16>,
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
      active_data_port: None,
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
        Command::Pasv => return self.pasv().await,
        Command::Epsv(proto) => return self.epsv(proto).await,
        Command::Port(port) => {
          self.active_data_port = Some(port);
          return self.send(Answer::new(ResultCode::Ok, &format!("Data port is now {}", port))).await;
        }
        Command::Pwd => {
          let msg = self.cwd.to_str().unwrap_or("").to_string(); // small trick
          if !msg.is_empty() {
            let message = format!("\"{}\" ", msg);
            return self.send(Answer::new(ResultCode::PATHNAMECreated, &message)).await;
          } else {
            return self.send(Answer::new(ResultCode::FileNotFound, "No such file or directory")).await;
          }
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
          //return Ok((self.send(Answer::new(ResultCode::Ok, "Okay."))).await?);
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
      Command::Auth => {
        //warn!("Auth not implemented");
        self.send(Answer::new(ResultCode::CommandNotImplemented, "Not implemented")).await?
      }
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
                  &format!("Login OK, password needed for {}", name.unwrap()),
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

  async fn initiate_data_connection(&mut self) -> Result<()> {
    // only open when we are in active mode
    if let Some(port) = self.active_data_port {
      let mut addr = self.remote_addr;
      addr.set_port(port);
      let stream = match TcpStream::connect(addr).await {
        Ok(stream) => stream,
        Err(e) => {
          error!("Connection failed to {}, {}", addr, e);
          self.send(Answer::new(ResultCode::CantOpenDataConnection, "Unable to connect")).await?;
          return Ok(());
        }
      };
      let (r, w) = stream.into_split();
      self.data_writer = Some(w);
      self.data_reader = Some(r);
    }
    Ok(())
  }

  fn close_data_connection(&mut self) {
    self.data_reader = None;
    self.data_writer = None;
  }

  fn complete_path(&self, path: &Path) -> result::Result<PathBuf, io::Error> {
    let directory = self.server_root.join(if path.has_root() {
      path.iter().skip(1).clone().collect()
    } else {
      path.to_path_buf()
    });
    let dir = directory.canonicalize();
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
      let res = self.complete_path(&parent);
      if let Ok(mut dir) = res {
        if dir.is_dir() {
          let filename = get_filename(fullpath);
          if let Some(filename) = filename {
            dir.push(filename);
            if create_dir(dir).is_ok() {
              self.send(Answer::new(ResultCode::PATHNAMECreated, "Folder successfully created!")).await?;
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
    let res = self.complete_path(&path);
    if let Ok(dir) = res {
      if remove_dir_all(dir).is_ok() {
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
    let res = self.complete_path(&path);
    if let Ok(f) = res {
      if remove_file(f).is_ok() {
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
    let res = self.complete_path(&path);
    if let Ok(dir) = res {
      let res = self.strip_prefix(dir);
      if let Ok(prefix) = res {
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
    self.initiate_data_connection().await?;
    if self.data_writer.is_some() {
      let path = self.cwd.join(path.unwrap_or_default());
      let directory = PathBuf::from(&path);
      let res = self.complete_path(&directory);
      if let Ok(path) = res {
        self
          .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to list directory..."))
          .await?;
        let mut out = vec![];
        if path.is_dir() {
          if let Ok(dir) = read_dir(path) {
            for entry in dir.flatten() {
              add_file_info(entry.path(), &mut out);
            }
          } else {
            self
              .send(Answer::new(ResultCode::InvalidParameterOrArgument, "No such file or directory"))
              .await?;
            return Ok(());
          }
        } else {
          add_file_info(path, &mut out);
        }
        self.send_data(out).await?;
      } else {
        warn!("No such file or directory");
        self
          .send(Answer::new(ResultCode::InvalidParameterOrArgument, "No such file or directory"))
          .await?;
      }
    } else {
      warn!("No opened data connection");
      self.send(Answer::new(ResultCode::ConnectionClosed, "No opened data connection")).await?;
    }
    if self.data_writer.is_some() {
      self.close_data_connection();
      self.send(Answer::new(ResultCode::ClosingDataConnection, "Transfer done")).await?;
    }
    Ok(())
  }

  async fn nlst(&mut self, path: Option<PathBuf>) -> Result<()> {
    self.initiate_data_connection().await?;
    if self.data_writer.is_some() {
      let res = self.complete_path(&self.cwd);
      if let Ok(cwd) = res {
        let mut out = vec![];

        let pattern = if let Some(p) = path {
          if p.is_absolute() {
            self
              .send(Answer::new(ResultCode::InvalidParameterOrArgument, "No such file or directory"))
              .await?;
            self.close_data_connection();
            return Ok(());
          }
          cwd.join(p)
        } else {
          cwd.join("*")
        };

        let matches = match pattern.to_str().map(|s| glob(s).map_err(|_| Error::from("Invalid glob pattern"))) {
          Some(Ok(matches)) => matches,
          _ => {
            self
              .send(Answer::new(ResultCode::InvalidParameterOrArgument, "Invalid pattern specified"))
              .await?;
            return Ok(());
          }
        };

        self
          .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to list directory..."))
          .await?;

        for entry in matches {
          match entry {
            Ok(path) => match path.canonicalize() {
              Ok(p) => {
                if p.starts_with(&self.server_root) {
                  add_file_info_nlst(path, &mut out);
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

        self.send_data(out).await?;
      } else {
        self.send(Answer::new(ResultCode::FileNotFound, "No such file or directory")).await?;
      }
    } else {
      self.send(Answer::new(ResultCode::ConnectionClosed, "No opened data connection")).await?;
    }
    if self.data_writer.is_some() {
      self.close_data_connection();
      self.send(Answer::new(ResultCode::ClosingDataConnection, "Transfer done")).await?;
    }
    Ok(())
  }

  async fn pasv(&mut self) -> Result<()> {
    if self.data_writer.is_some() {
      self.send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Already listening...")).await?;
      return Ok(());
    }
    self.active_data_port = None;
    //let port = if let Some(port) = self.data_port { port } else { 0 };
    let port = 0; // auto configure port
    let mut addr = self.local_addr;
    addr.set_port(port);

    // If it's a IPv4 address but mapped into v6 as defined in IETF RFC 4291 section 2.5.5.2,
    // use the canonical value.
    match &addr.ip().to_canonical() {
      IpAddr::V4(v4addr) => {
        let listener = TcpListener::bind(&addr).await?;
        let port = listener.local_addr()?.port();
        let octets = v4addr.octets();

        self
          .send(Answer::new(
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
          ))
          .await?;

        debug!("Waiting for data clients on port {}...", port);
        {
          let (stream, _addr) = listener.accept().await?;
          let (r, w) = stream.into_split();
          self.data_writer = Some(w);
          self.data_reader = Some(r);
        }
      }
      IpAddr::V6(_v6addr) => {
        return Err(Error::Msg("PASV is not possible for IPv6 connections.".into()));
      }
    }
    Ok(())
  }

  async fn epsv(&mut self, proto: Option<String>) -> Result<()> {
    //let port = if let Some(port) = self.data_port { port } else { 0 };
    let port = 0;

    if let Some(proto) = proto {
      if proto.to_uppercase().eq("ALL") {
        self.data_writer = None;
        return Ok(());
      }
    }

    if self.data_writer.is_some() {
      self.send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Already listening...")).await?;
      return Ok(());
    }

    self.active_data_port = None;

    let mut addr = self.local_addr;
    addr.set_port(port);
    let listener = TcpListener::bind(&addr).await?;
    let port = listener.local_addr()?.port();

    self
      .send(Answer::new(ResultCode::ExtendedEnteringPassiveMode, &format!(" (|||{}|)", port,)))
      .await?;

    debug!("Waiting for data clients on port {}...", port);
    {
      let (stream, _addr) = listener.accept().await?;
      let (r, w) = stream.into_split();

      self.data_writer = Some(w);
      self.data_reader = Some(r);
    }

    Ok(())
  }

  async fn quit(&mut self) -> Result<()> {
    if self.data_writer.is_some() {
      unimplemented!();
    } else {
      self
        .send(Answer::new(ResultCode::ServiceClosingControlConnection, "Closing connection..."))
        .await?;
      self.writer.close().await?;
    }
    Ok(())
  }

  async fn retr(&mut self, path: PathBuf) -> Result<()> {
    self.initiate_data_connection().await?;
    // TODO: check if multiple data connection can be opened at the same time.
    if self.data_writer.is_some() {
      let path = self.cwd.join(path);
      let res = self.complete_path(&path);
      if let Ok(path) = res {
        if path.is_file() {
          self
            .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to send file..."))
            .await?;
          let mut file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(e) => {
              error!("Failed to open file {:?}, {}", path, e);
              self
                .send(Answer::new(
                  ResultCode::LocalErrorInProcessing,
                  &format!("\"{}\" doesn't exist", path.to_str().ok_or_else(|| Error::Msg("No path".to_string()))?),
                ))
                .await?;
              return Ok(());
            }
          };
          if let Some(writer) = &mut self.data_writer {
            tokio::io::copy(&mut file, writer).await?;
          } else {
            return Err(Error::from("Trying to send data but no data connection is open."));
          }
        } else {
          warn!("No path, not exist");
          self
            .send(Answer::new(
              ResultCode::LocalErrorInProcessing,
              &format!("\"{}\" doesn't exist", path.to_str().ok_or_else(|| Error::Msg("No path".to_string()))?),
            ))
            .await?;
        }
      } else {
        warn!("No path");
        self
          .send(Answer::new(
            ResultCode::LocalErrorInProcessing,
            &format!("\"{}\" doesn't exist", path.to_str().ok_or_else(|| Error::Msg("No path".to_string()))?),
          ))
          .await?;
      }
    } else {
      warn!("No open data connection");
      self.send(Answer::new(ResultCode::ConnectionClosed, "No opened data connection")).await?;
    }
    if self.data_writer.is_some() {
      self.close_data_connection();
      self.send(Answer::new(ResultCode::ClosingDataConnection, "Transfer done")).await?;
    }
    Ok(())
  }

  async fn stor(&mut self, path: PathBuf) -> Result<()> {
    self.initiate_data_connection().await?;
    if self.data_reader.is_some() {
      if invalid_path(&path) {
        return Err(Error::Io(io::ErrorKind::PermissionDenied.into()));
      }
      let path = self.cwd.join(path);
      self
        .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to send file..."))
        .await?;
      let data = self.receive_data().await?;
      info!("Received file: {:?}, {} bytes", path, data.len());
      self.close_data_connection();
      match self.put_file(path, Arc::new(data)).await {
        Ok(_) => {
          self.send(Answer::new(ResultCode::ClosingDataConnection, "Transfer done")).await?;
        }
        Err(e) => {
          error!("Failed STOR command: {:?}", e);
          self.send(Answer::new(ResultCode::LocalErrorInProcessing, "Failed to store the file")).await?;
        }
      }
    } else {
      warn!("No opened data connection");
      self.send(Answer::new(ResultCode::ConnectionClosed, "No opened data connection")).await?;
    }
    Ok(())
  }

  /// Put file directly or delegate to a filter
  async fn put_file(&mut self, path: PathBuf, content: Arc<Vec<u8>>) -> Result<()> {
    let path = PathBuf::from(&self.server_root).join(path.iter().skip(1).collect::<PathBuf>());
    let handled = self.env.stor_file(&path, content.clone())?;
    if !handled {
      let mut file = File::create(path)?;
      file.write_all(&content)?;
    }
    Ok(())
  }

  /// Receive data from a data connection and take out the reader
  async fn receive_data(&mut self) -> Result<Vec<u8>> {
    let mut reader = self.data_reader.take().ok_or_else(|| Error::from("No data reader"))?;
    let mut buf = Vec::new();
    tokio::io::copy(&mut reader, &mut buf).await?;
    Ok(buf)
  }

  /// Send a control answer to the client
  async fn send(&mut self, answer: Answer) -> Result<()> {
    self.writer.send(answer).await?;
    Ok(())
  }

  /// Send bytes to the client
  async fn send_data(&mut self, data: Vec<u8>) -> Result<()> {
    if let Some(writer) = &mut self.data_writer {
      writer.write_all(&data).await?;
      Ok(())
    } else {
      Err(Error::from("Trying to send data but no data connection is open."))
    }
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
    client.handle_cmd(cmd).await?;
  }

  debug!("Client {} closed connection", remote_addr);
  Ok(())
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

fn prefix_slash(path: &mut PathBuf) {
  if !path.is_absolute() {
    *path = Path::new("/").join(&path);
  }
}

// If an error occurs when we try to get file's information, we just return and don't send its info.
fn add_file_info(path: PathBuf, out: &mut Vec<u8>) {
  let extra = if path.is_dir() { "/" } else { "" };
  let is_dir = if path.is_dir() { "d" } else { "-" };

  let meta = match ::std::fs::metadata(&path) {
    Ok(meta) => meta,
    _ => return,
  };
  let time: chrono::DateTime<Utc> = meta.modified().unwrap().into();
  #[cfg(unix)]
  let file_size = meta.size();
  #[cfg(windows)]
  let file_size = meta.file_size();
  let path = match path.to_str() {
    Some(path) => match path.split('/').last() {
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
fn add_file_info_nlst(path: PathBuf, out: &mut Vec<u8>) {
  let extra = if path.is_dir() { "/" } else { "" };
  let path = match path.to_str() {
    Some(path) => match path.split('/').last() {
      Some(path) => path,
      _ => return,
    },
    _ => return,
  };

  let file_str = format!("{path}{extra}\r\n", path = path, extra = extra);
  out.extend(file_str.as_bytes());
  debug!("==> {:?}", &file_str);
}
