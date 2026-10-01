//! `/api/forward/:port` — a TCP port on the daemon's own host, carried over a WebSocket.
//!
//! This is `ssh -L` for a Forge connection: the desktop app listens on a local port and pipes each
//! connection through here, so a dev server the operator (or an agent) started on the server — the
//! Portal SPA on :5173, an API on :8080 — opens on the operator's machine as if it ran there. It
//! goes over the same authenticated connection as everything else, so it works through a Cloudflare
//! tunnel or a reverse proxy, where an SSH port is usually not reachable at all.
//!
//! Only loopback on the server is reachable, and only above the privileged range: this forwards to
//! services the operator runs for review, not to anything on the server's network.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn forward_port(Path(port): Path<u16>, ws: WebSocketUpgrade) -> Response {
    if port < 1024 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Only ports 1024 and above can be forwarded" })),
        )
            .into_response();
    }
    ws.on_upgrade(move |socket| relay(socket, port))
}

async fn relay(socket: WebSocket, port: u16) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let tcp = match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(tcp) => tcp,
        Err(e) => {
            let _ = ws_tx
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: format!("Nothing is listening on port {port} on the server: {e}").into(),
                })))
                .await;
            return;
        }
    };
    let _ = tcp.set_nodelay(true);
    let (mut tcp_rx, mut tcp_tx) = tcp.into_split();

    let upstream = async {
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
    let downstream = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match tcp_rx.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_tx.send(Message::Binary(buf[..n].to_vec())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = ws_tx.send(Message::Close(None)).await;
    };
    tokio::join!(upstream, downstream);
}
