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
use std::time::Duration;
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
  // The paths of the clients are resolved and checked against the root
  let server_root = tokio::fs::canonicalize(&server_root)
    .await
    .map_err(|e| io::Error::new(e.kind(), format!("Invalid server root {}: {}", server_root.display(), e)))?;
  let port = config.server_port;
  let addr = SocketAddr::new(config.server_addr, port);
  let listener = TcpListener::bind(&addr).await?;

  info!("Listening for clients on port {}...", port);
  loop {
    let (stream, addr) = match listener.accept().await {
      Ok(client) => client,
      Err(err) => {
        // E.g. no free file descriptor. The server continues with the next client.
        error!("Failed to accept a client: {}", err);
        tokio::time::sleep(Duration::from_millis(100)).await;
        continue;
      }
    };
    info!("New client connected: {}", addr);
    handle.spawn(handle_client(addr, stream, handle.clone(), server_root.clone(), config.clone(), env.clone()));
  }
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
