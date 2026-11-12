// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use chrono::{DateTime, TimeDelta, Utc};
use futures::TryStreamExt;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use glob::Pattern;
use log::{debug, error, info, warn};
use std::io;
use std::mem;
use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf, StripPrefixError};
use std::result;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::fs::{File, canonicalize, create_dir, metadata, read_dir, remove_dir, remove_file, symlink_metadata};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::time::timeout;
use tokio_util::codec::{Decoder, Framed};

#[cfg(unix)]
use std::os::unix::prelude::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use crate::codec::FtpCodec;
use crate::command::{Command, ParseError, TransferType};
use crate::config::{Config, FtpCallback};
use crate::error::Result;
use crate::ftp::{Answer, ResultCode};

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
    match &cmd {
      Command::Pass(_) => debug!("Received command: Pass(***)"),
      _ => debug!("Received command: {:?}", cmd),
    }
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
        // Any password is fine, but for the anonymous user only
        if self.config.anonymous && self.name.as_deref() == Some("anonymous") {
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
      ParseError::TooLong => Answer::new(ResultCode::UnknownCommand, "Command line too long"),
    };
    self.send(answer).await
  }

  /// Open the data connection of a transfer: connect to the client in active mode,
  /// accept the connection of the client in passive mode.
  /// Returns false if there is no data connection.
  async fn initiate_data_connection(&mut self) -> bool {
    self.close_data_connection();
    let data_timeout = self.config.data_timeout;
    let stream = match mem::replace(&mut self.data_mode, DataMode::None) {
      DataMode::None => return false,
      DataMode::Active(port) => {
        // The port is used for further transfers, too
        self.data_mode = DataMode::Active(port);
        let mut addr = self.remote_addr;
        addr.set_port(port);
        timeout(data_timeout, TcpStream::connect(addr))
          .await
          .map_err(io::Error::from)
          .and_then(|res| res)
      }
      DataMode::Passive(listener) => timeout(data_timeout, accept_from(&listener, self.remote_addr.ip()))
        .await
        .map_err(io::Error::from)
        .and_then(|res| res),
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

  /// Path of the client as path from the root of the server: "/" and the names below it.
  /// The path is relative to the working directory or starts at the root, ".." stops at
  /// the root. None for a path with a prefix like "C:" on Windows.
  fn virtual_path(&self, path: &Path) -> Option<PathBuf> {
    let mut result = self.cwd.clone();
    for component in path.components() {
      match component {
        Component::Prefix(_) => return None,
        Component::RootDir => result = PathBuf::from("/"),
        Component::CurDir => {}
        Component::ParentDir => {
          result.pop();
        }
        Component::Normal(name) => result.push(name),
      }
    }
    Some(result)
  }

  /// Path on the server for a virtual path, links are resolved.
  /// It's an error if the result is not below the server root.
  async fn real_path(&self, virtual_path: &Path) -> io::Result<PathBuf> {
    let names = virtual_path.components().filter(|c| matches!(c, Component::Normal(_)));
    let path = canonicalize(self.server_root.join(names.collect::<PathBuf>())).await?;
    if path.starts_with(&self.server_root) {
      Ok(path)
    } else {
      Err(io::ErrorKind::PermissionDenied.into())
    }
  }

  /// Path on the server for the path of an existing file or directory of the client
  async fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
    let virtual_path = self.virtual_path(path).ok_or(io::ErrorKind::PermissionDenied)?;
    self.real_path(&virtual_path).await
  }

  /// Path on the server for the path of a file or directory the client creates.
  /// The directory of it must exist.
  async fn resolve_new(&self, path: &Path) -> io::Result<PathBuf> {
    let virtual_path = self.virtual_path(path).ok_or(io::ErrorKind::PermissionDenied)?;
    let (Some(parent), Some(name)) = (virtual_path.parent(), virtual_path.file_name()) else {
      return Err(io::ErrorKind::PermissionDenied.into());
    };
    let path = self.real_path(parent).await?.join(name);
    // A link at the place of the new file is followed, it must not lead out of the root
    if symlink_metadata(&path).await.is_ok_and(|meta| meta.is_symlink()) {
      match self.real_path(&virtual_path).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(io::ErrorKind::PermissionDenied.into()),
        Err(e) => return Err(e),
      }
    }
    Ok(path)
  }

  async fn mkd(&mut self, path: PathBuf) -> Result<()> {
    if let Ok(dir) = self.resolve_new(&path).await {
      if create_dir(&dir).await.is_ok() {
        let mut created = self.strip_prefix(dir).unwrap_or_default();
        prefix_slash(&mut created);
        let message = format!("{} created", quote_path(&created));
        self.send(Answer::new(ResultCode::PATHNAMECreated, &message)).await?;
        return Ok(());
      }
    }
    let msg = format!("Couldn't create folder: {}", path.to_string_lossy());
    warn!("{}", msg);
    self.send(Answer::new(ResultCode::FileNotFound, &msg)).await?;
    Ok(())
  }

  async fn rmd(&mut self, directory: PathBuf) -> Result<()> {
    let res = self.resolve(&directory).await;
    if let Ok(dir) = res {
      // Only an empty directory is removed, and never the root
      if dir != self.server_root && remove_dir(dir).await.is_ok() {
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
    let res = self.resolve(&file).await;
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
    let res = self.resolve(&directory).await;
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
    let Ok(path) = self.resolve(&path.unwrap_or_default()).await else {
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
    let names = match self.nlst_names(&path.unwrap_or_default()).await {
      Ok(names) => names,
      Err(answer) => return Ok(answer),
    };
    let mut out = vec![];
    for name in names {
      out.extend(name.as_bytes());
      out.extend(b"\r\n");
    }
    self
      .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to list directory..."))
      .await?;
    Ok(self.send_data(out).await)
  }

  /// Names for NLST: the entries of the directory, or the entries in the directory of
  /// the path that match its last component as pattern. A name starts with the
  /// directory as the client gave it, so it is valid as argument of other commands.
  async fn nlst_names(&self, path: &Path) -> result::Result<Vec<String>, Answer> {
    let not_found = || Answer::new(ResultCode::FileNotFound, "No such file or directory");
    let mut dir = self.resolve(path).await;
    if let Ok(path) = &dir {
      if !metadata(path).await.is_ok_and(|meta| meta.is_dir()) {
        dir = Err(io::ErrorKind::NotADirectory.into());
      }
    }
    let (dir, prefix, pattern) = match dir {
      Ok(dir) => (dir, path, None),
      Err(_) => {
        let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(not_found)?;
        let pattern = Pattern::new(name).map_err(|_| Answer::new(ResultCode::InvalidParameterOrArgument, "Invalid pattern specified"))?;
        let parent = path.parent().unwrap_or(Path::new(""));
        let dir = self.resolve(parent).await.map_err(|_| not_found())?;
        (dir, parent, Some(pattern))
      }
    };
    let prefix = match prefix.to_string_lossy() {
      prefix if prefix.is_empty() => String::new(),
      prefix => format!("{}/", prefix.trim_end_matches('/')),
    };
    let mut entries = read_dir(&dir).await.map_err(|_| not_found())?;
    let mut names = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
      // A name that is not UTF-8 can't be sent
      if let Ok(name) = entry.file_name().into_string() {
        if pattern.as_ref().is_none_or(|pattern| pattern.matches(&name)) {
          names.push(format!("{}{}", prefix, name));
        }
      }
    }
    names.sort();
    Ok(names)
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
    let not_found = Answer::new(ResultCode::FileNotFound, &format!("\"{}\" doesn't exist", path.display()));
    let Ok(path) = self.resolve(&path).await else {
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
    let data_timeout = self.config.data_timeout;
    let res = match &mut self.data_writer {
      Some(writer) => copy_data(&mut file, writer, data_timeout).await,
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
    let path = match self.resolve_new(&path).await {
      Ok(path) => path,
      Err(e) => {
        warn!("Invalid path for STOR: {:?}, {}", path, e);
        return Ok(match e.kind() {
          io::ErrorKind::NotFound => Answer::new(ResultCode::FileNotFound, "No such directory"),
          _ => Answer::new(ResultCode::FileNameNotAllowed, "File name not allowed"),
        });
      }
    };
    self
      .send(Answer::new(ResultCode::DataConnectionAlreadyOpen, "Starting to send file..."))
      .await?;
    let data = match self.receive_data().await {
      Ok(data) => data,
      Err(e) if e.kind() == io::ErrorKind::FileTooLarge => {
        warn!("Upload of {:?} exceeds the maximum size", path);
        return Ok(Answer::new(ResultCode::ExceededStorageAllocation, "File exceeds the maximum upload size"));
      }
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
    let handled = self.env.stor_file(&path, content.clone()).await?;
    if !handled {
      tokio::fs::write(&path, content.as_ref()).await?;
    }
    Ok(())
  }

  /// Receive data from a data connection and take out the reader
  async fn receive_data(&mut self) -> io::Result<Vec<u8>> {
    let reader = self.data_reader.take().ok_or(io::ErrorKind::NotConnected)?;
    let limit = self.config.max_upload_size;
    let mut buf = Vec::new();
    // One byte more than the limit shows that the file is too large
    copy_data(reader.take(limit.saturating_add(1)), &mut buf, self.config.data_timeout).await?;
    if buf.len() as u64 > limit {
      return Err(io::ErrorKind::FileTooLarge.into());
    }
    Ok(buf)
  }

  /// Send a control answer to the client
  async fn send(&mut self, answer: Answer) -> Result<()> {
    self.writer.send(answer).await?;
    Ok(())
  }

  /// Send bytes to the client, returns the final reply of the transfer
  async fn send_data(&mut self, data: Vec<u8>) -> Answer {
    let data_timeout = self.config.data_timeout;
    let res = match &mut self.data_writer {
      Some(writer) => copy_data(data.as_slice(), writer, data_timeout).await,
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
  let (mut writer, mut reader) = FtpCodec::default().framed(stream).split();
  writer.send(Answer::new(ResultCode::ServiceReadyForNewUser, &config.greeting)).await?;

  let idle_timeout = config.idle_timeout;
  let mut client = Client::new(handle, writer, server_root, config, env, local_addr, remote_addr);
  loop {
    let Ok(cmd) = timeout(idle_timeout, reader.try_next()).await else {
      info!("Client {} sent no command for {} seconds", remote_addr, idle_timeout.as_secs());
      client
        .send(Answer::new(ResultCode::ServiceNotAvailable, "Idle timeout, closing connection"))
        .await?;
      break;
    };
    let Some(cmd) = cmd? else {
      break;
    };
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

/// Tell a client that the server has no free session
pub(crate) async fn refuse(stream: TcpStream) {
  let mut writer = FtpCodec::default().framed(stream);
  let answer = Answer::new(ResultCode::ServiceNotAvailable, "Too many connections, try again later");
  // The reply is a courtesy, the connection is closed in any case
  let _ = timeout(Duration::from_secs(5), writer.send(answer)).await;
}

/// Accept the data connection of the client. Connections of other hosts are closed,
/// they could read or supply the data of the transfer.
async fn accept_from(listener: &TcpListener, client: IpAddr) -> io::Result<TcpStream> {
  loop {
    let (stream, addr) = listener.accept().await?;
    if addr.ip().to_canonical() == client.to_canonical() {
      return Ok(stream);
    }
    warn!("Closed data connection of {}, the client is {}", addr, client);
  }
}

/// Copy all data from the reader to the writer.
/// It's an error if a read or a write takes longer than the timeout.
async fn copy_data<R, W>(mut reader: R, mut writer: W, data_timeout: Duration) -> io::Result<()>
where
  R: AsyncRead + Unpin,
  W: AsyncWrite + Unpin,
{
  let mut buf = vec![0; 64 * 1024];
  loop {
    let len = timeout(data_timeout, reader.read(&mut buf)).await??;
    if len == 0 {
      break;
    }
    timeout(data_timeout, writer.write_all(&buf[..len])).await??;
  }
  timeout(data_timeout, writer.flush()).await??;
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
  // A time that chrono can't represent is shown as the epoch
  let time = meta
    .modified()
    .ok()
    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
    .and_then(|since| DateTime::from_timestamp(i64::try_from(since.as_secs()).ok()?, 0))
    .unwrap_or(DateTime::UNIX_EPOCH);
  #[cfg(unix)]
  let file_size = meta.size();
  #[cfg(windows)]
  let file_size = meta.file_size();
  // A name that is not UTF-8 can't be sent
  let path = match path.file_name().and_then(|name| name.to_str()) {
    Some(name) => name,
    _ => return,
  };
  // TODO: maybe improve how we get rights in here?
  let rights = if meta.permissions().readonly() { "r--r--r--" } else { "rw-rw-rw-" };

  // Like "ls -l": the time of day for the last six months, the year for other times
  let now = Utc::now();
  let time = if time <= now && now - time < TimeDelta::days(180) {
    time.format("%b %e %H:%M")
  } else {
    time.format("%b %e  %Y")
  };

  let file_str = format!(
    "{is_dir}{rights} {links} {owner} {group} {size} {time} {path}{extra}\r\n",
    is_dir = is_dir,
    rights = rights,
    links = 1,           // number of links
    owner = "anonymous", // owner name
    group = "anonymous", // group name
    size = file_size,
    time = time,
    path = path,
    extra = extra
  );
  out.extend(file_str.as_bytes());
  //debug!("==> {:?}", &file_str);
}
