//! Reaching a remote daemon through SSH instead of exposing it on the internet.
//!
//! [`Tunnel::start`] binds a loopback port on this machine and forwards every connection to it over
//! one SSH session to `127.0.0.1:<remote_port>` on the server (`ssh -L`, in-process). The rest of the
//! app is pointed at `http://127.0.0.1:<local_port>` and never knows SSH is involved: the command
//! surface, the WebSocket bridge and the SSE streams all ride the same forward.
//!
//! The session is (re)established lazily, on the next forwarded connection after it drops, so a laptop
//! that sleeps or changes networks recovers on its own the first time anything talks to the daemon.
//! SSH keepalives notice a dead session well before a proxy-style idle cut would.
//!
//! Host keys are trust-on-first-use: the fingerprint seen on the first successful connect is saved
//! with the connection, and a later mismatch refuses to connect rather than silently trusting a new
//! machine.

use russh::client;
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const DEFAULT_SSH_PORT: u16 = 22;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// What the operator types, plus the host key pinned on first connect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// The daemon's port on the server, reached as `127.0.0.1:<remote_port>` from there.
    pub remote_port: u16,
    /// `SHA256:…` of the server's host key, once one has been seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<String>,
}

impl SshSettings {
    /// For display: `user@host` or `user@host:port`, never the password.
    pub fn label(&self) -> String {
        if self.port == DEFAULT_SSH_PORT {
            format!("ssh://{}@{}", self.username, self.host)
        } else {
            format!("ssh://{}@{}:{}", self.username, self.host, self.port)
        }
    }
}

/// Accepts `host`, `host:port` or `user@host[:port]`; a user in the address wins over `username`.
pub fn parse_address(input: &str, username: &str) -> Result<(String, u16, String), String> {
    let trimmed = input
        .trim()
        .trim_start_matches("ssh://")
        .trim_end_matches('/');
    let (user, rest) = match trimmed.rsplit_once('@') {
        Some((user, rest)) => (user.trim().to_string(), rest),
        None => (username.trim().to_string(), trimmed),
    };
    if rest.is_empty() {
        return Err("Enter the server's SSH address".to_string());
    }
    if user.is_empty() {
        return Err("Enter the SSH username".to_string());
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (
            host.to_string(),
            port.parse::<u16>()
                .map_err(|_| format!("\"{port}\" is not a valid SSH port"))?,
        ),
        _ => (rest.to_string(), DEFAULT_SSH_PORT),
    };
    if host.is_empty() {
        return Err("The SSH address has no host".to_string());
    }
    Ok((host, port, user))
}

struct Client {
    expected: Option<String>,
    seen: Arc<StdMutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fingerprint.clone());
        Ok(self.expected.as_ref().is_none_or(|e| *e == fingerprint))
    }
}

/// Connects and logs in; answers with the session and the host key it presented.
async fn open_session(settings: &SshSettings) -> Result<(client::Handle<Client>, String), String> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let seen = Arc::new(StdMutex::new(None));
    let handler = Client {
        expected: settings.host_key.clone(),
        seen: Arc::clone(&seen),
    };
    let address = (settings.host.as_str(), settings.port);
    let target = format!("{}:{}", settings.host, settings.port);

    let connected =
        tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, address, handler))
            .await
            .map_err(|_| format!("Timed out reaching {target} over SSH"))?;
    let seen_key = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut session = match connected {
        Ok(session) => session,
        Err(e) => {
            return Err(match (&settings.host_key, &seen_key) {
                (Some(expected), Some(seen)) if expected != seen => format!(
                    "{target}'s SSH host key has changed (expected {expected}, got {seen}). \
                     If the server was reinstalled, disconnect and connect again to trust the new key."
                ),
                _ => format!("Could not connect to {target} over SSH: {e}"),
            });
        }
    };
    let host_key = seen_key.ok_or_else(|| format!("{target} presented no SSH host key"))?;

    let auth = session
        .authenticate_password(settings.username.clone(), settings.password.clone())
        .await
        .map_err(|e| format!("SSH login to {target} failed: {e}"))?;
    if !auth.success() {
        return Err(format!(
            "SSH login to {target} as {} was refused: wrong username or password, or the server does not allow password logins",
            settings.username
        ));
    }
    Ok((session, host_key))
}

/// A running local forward. Dropping it stops forwarding and closes the session.
pub struct Tunnel {
    pub local_port: u16,
    /// The host key the first session presented, for pinning on first use.
    pub host_key: Option<String>,
    /// Why the first session failed, if it did. The forward still runs and retries on demand.
    pub first_error: Option<String>,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Tunnel {
    /// Binds the local port and tries the first session, so a wrong password or unreachable host
    /// is reported in [`Tunnel::first_error`]. Only failing to bind is an error: the port has to
    /// exist for the app to be pointed at it, and a session that is down now is retried on demand.
    pub async fn start(settings: SshSettings) -> Result<Tunnel, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("Could not open a local port for the SSH tunnel: {e}"))?;
        let local_port = listener.local_addr().map_err(|e| e.to_string())?.port();

        let (session, host_key, first_error) = match open_session(&settings).await {
            Ok((session, key)) => (Some(session), Some(key), None),
            Err(e) => (None, None, Some(e)),
        };
        let settings = SshSettings {
            host_key: settings.host_key.clone().or_else(|| host_key.clone()),
            ..settings
        };
        let session = Arc::new(Mutex::new(session));
        let task = tauri::async_runtime::spawn(async move {
            loop {
                let Ok((inbound, peer)) = listener.accept().await else {
                    continue;
                };
                tauri::async_runtime::spawn(forward(
                    inbound,
                    peer,
                    Arc::clone(&session),
                    settings.clone(),
                ));
            }
        });
        Ok(Tunnel {
            local_port,
            host_key,
            first_error,
            task,
        })
    }
}

/// Carries one local connection to the daemon, reopening the session first if it has dropped.
async fn forward(
    mut inbound: tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    session: Arc<Mutex<Option<client::Handle<Client>>>>,
    settings: SshSettings,
) {
    let channel = {
        let mut guard = session.lock().await;
        if guard.as_ref().is_none_or(|s| s.is_closed()) {
            match open_session(&settings).await {
                Ok((fresh, _)) => *guard = Some(fresh),
                Err(e) => {
                    tracing::warn!("SSH tunnel reconnect failed: {e}");
                    *guard = None;
                    return;
                }
            }
        }
        let Some(active) = guard.as_ref() else { return };
        active
            .channel_open_direct_tcpip(
                "127.0.0.1",
                u32::from(settings.remote_port),
                peer.ip().to_string(),
                u32::from(peer.port()),
            )
            .await
    };
    match channel {
        Ok(channel) => {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut inbound, &mut stream).await;
        }
        Err(e) => tracing::warn!(
            "SSH tunnel could not reach 127.0.0.1:{} on the server: {e}",
            settings.remote_port
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_port_and_user_forms() {
        assert_eq!(
            parse_address("my-vps", "ryan").unwrap(),
            ("my-vps".into(), 22, "ryan".into())
        );
        assert_eq!(
            parse_address(" ryan@203.0.113.7:2222 ", "").unwrap(),
            ("203.0.113.7".into(), 2222, "ryan".into())
        );
        assert_eq!(
            parse_address("ssh://root@box/", "ignored").unwrap(),
            ("box".into(), 22, "root".into())
        );
    }

    #[test]
    fn rejects_missing_parts() {
        assert!(parse_address("", "ryan").is_err());
        assert!(parse_address("my-vps", " ").is_err());
        assert!(parse_address("my-vps:notaport", "ryan").is_err());
    }

    #[test]
    fn label_hides_the_default_port_and_the_password() {
        let mut s = SshSettings {
            host: "box".into(),
            port: 22,
            username: "ryan".into(),
            password: "secret".into(),
            remote_port: 5010,
            host_key: None,
        };
        assert_eq!(s.label(), "ssh://ryan@box");
        s.port = 2222;
        assert_eq!(s.label(), "ssh://ryan@box:2222");
    }
}
