// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

/*
 * FIXME: Filezilla says: Le serveur ne supporte pas les caractères non-ASCII.
 * FIXME: ftp cli says "WARNING! 71 bare linefeeds received in ASCII mode" when retrieving a file.
 */

use log::{debug, error, info};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;

use crate::client::client;
use crate::config::{Config, FtpCallback};

/// Start the server processing loop
pub async fn serve<T>(handle: Handle, server_root: PathBuf, config: Config, env: T) -> io::Result<()>
where
  T: FtpCallback + Clone + Send + Sync + 'static,
{
  let port = config.server_port;
  let addr = SocketAddr::new(config.server_addr, port);
  let listener = TcpListener::bind(&addr).await?;

  info!("Listening for clients on port {}...", port);
  loop {
    let (stream, addr) = listener.accept().await?;
    info!("New client connected: {}", addr);
    handle.spawn(handle_client(addr, stream, handle.clone(), server_root.clone(), config.clone(), env.clone()));
  }
  //Ok(())
}

/// Handle a single client connection
async fn handle_client<T>(addr: SocketAddr, stream: TcpStream, handle: Handle, server_root: PathBuf, config: Config, env: T)
where
  T: FtpCallback + Clone + Send + Sync + 'static,
{
  match client(stream, handle, server_root, config, env).await {
    Err(err) => {
      error!("Error while handling client: {}", err);
    }
    Ok(_) => {
      debug!("Client closed connection: {}", addr);
    }
  }
}
