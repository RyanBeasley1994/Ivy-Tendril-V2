//! Port forwards to the daemon's host: `ssh -L` over the Forge connection.
//!
//! [`forward`] listens on `127.0.0.1:<port>` here (the same port when it is free, the next free one
//! otherwise) and carries each connection through the daemon's `/api/forward/:port` WebSocket to
//! `127.0.0.1:<port>` on its host. So a dev server started on a remote server, by a review action or
//! by an agent in a plan chat, opens on this machine at a loopback URL, live reload included, without
//! an SSH port being reachable (a Cloudflare tunnel exposes none).
//!
//! Against a local daemon there is nothing to forward: the port is already here, and it is returned
//! unchanged.

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForward {
    /// The port on the server.
    pub remote_port: u16,
    /// Where it opens here.
    pub local_port: u16,
    /// `false` when the daemon is on this machine and nothing had to be forwarded.
    pub forwarded: bool,
}

struct Running {
    info: PortForward,
    task: tauri::async_runtime::JoinHandle<()>,
}

static FORWARDS: Mutex<BTreeMap<u16, Running>> = Mutex::new(BTreeMap::new());

fn is_remote() -> bool {
    crate::service::remote::is_active()
}

/// Every forward that is open, by server port.
pub fn list() -> Vec<PortForward> {
    FORWARDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(|r| r.info.clone())
        .collect()
}

pub fn stop(remote_port: u16) -> bool {
    match FORWARDS.lock().unwrap_or_else(|e| e.into_inner()).remove(&remote_port) {
        Some(running) => {
            running.task.abort();
            true
        }
        None => false,
    }
}

/// Opens (or returns the already open) forward of `remote_port`.
pub async fn forward(remote_port: u16) -> Result<PortForward, String> {
    if remote_port < 1024 {
        return Err("Only ports 1024 and above can be forwarded".into());
    }
    if !is_remote() {
        return Ok(PortForward { remote_port, local_port: remote_port, forwarded: false });
    }
    if let Some(existing) = FORWARDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&remote_port)
        .filter(|r| !r.task.inner().is_finished())
        .map(|r| r.info.clone())
    {
        return Ok(existing);
    }

    // The same port when it is free, so the URL the server printed works unchanged.
    let listener = match TcpListener::bind(("127.0.0.1", remote_port)).await {
        Ok(l) => l,
        Err(_) => TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("Could not open a local port: {e}"))?,
    };
    let local_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let info = PortForward { remote_port, local_port, forwarded: true };

    let task = tauri::async_runtime::spawn(async move {
        loop {
            let Ok((inbound, _)) = listener.accept().await else { continue };
            let _ = inbound.set_nodelay(true);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = carry(inbound, remote_port).await {
                    tracing::warn!("Port forward :{remote_port}: {e}");
                }
            });
        }
    });
    FORWARDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(remote_port, Running { info: info.clone(), task });
    tracing::info!("Forwarding 127.0.0.1:{local_port} to :{remote_port} on the server");
    Ok(info)
}

/// One connection: a fresh WebSocket per TCP connection, with the current session token (it is
/// refreshed in the background, so one captured at start would expire).
async fn carry(inbound: TcpStream, remote_port: u16) -> Result<(), String> {
    let master = crate::daemon::read_master(&crate::daemon::resolve_tendril_home())?;
    let scheme = if master.scheme == "https" { "wss" } else { "ws" };
    let url = format!("{scheme}://{}:{}/api/forward/{remote_port}", master.host, master.port);
    let mut request = url.as_str().into_client_request().map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", master.secret)
            .parse()
            .map_err(|_| "Invalid session token".to_string())?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("Could not reach the server's forward: {e}"))?;

    use futures_util::{SinkExt, StreamExt};
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (mut tcp_rx, mut tcp_tx) = inbound.into_split();

    let up = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match tcp_rx.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = ws_tx.send(Message::Close(None)).await;
    };
    let down = async {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Binary(bytes) => {
                    if tcp_tx.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = tcp_tx.shutdown().await;
    };
    tokio::join!(up, down);
    Ok(())
}
