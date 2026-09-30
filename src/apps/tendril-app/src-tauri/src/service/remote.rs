//! Pointing the desktop app at a Tendril daemon on another machine.
//!
//! Every command reaches the daemon through `daemon::read_master`, which normally reads the local
//! `.master` file. When a remote connection is saved, [`master_override`] answers in its place with
//! the remote origin and a session token, so the whole command surface, the WebSocket bridge and the
//! change stream follow without each one knowing a remote exists.
//!
//! The daemon secret never leaves the remote host. What the app holds is a session token from
//! `POST /api/auth/login`, which the daemon issues for 15 minutes, so a background task logs in again
//! every [`REFRESH_EVERY`] with the saved password. That makes the password itself the thing kept on
//! disk: it sits in `<TendrilHome>/remote-connection.json`, owner-only on Unix, next to the `.master`
//! secret and `config.yaml` credentials that folder already holds.
//!
//! With [`SavedConnection::ssh`] set, the daemon is not reached at `url` at all: an SSH tunnel
//! (`service::ssh_tunnel`) forwards a loopback port to it and the origin is that port, so the daemon
//! can stay bound to the server's loopback, off the internet.
//!
//! The connection is chosen at startup. Connecting or disconnecting saves the choice and restarts
//! the app, because the bridges are wired up once in `setup`.

use crate::daemon::MasterInfo;
use crate::service::ssh_tunnel::{SshSettings, Tunnel};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

const FILE_NAME: &str = "remote-connection.json";
/// Comfortably inside the daemon's 15-minute `SESSION_TOKEN_TTL_SECONDS`.
const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);
const RETRY_EVERY: Duration = Duration::from_secs(15);
/// `tendril run`'s default, assumed when the operator types a bare host or IP.
pub const DEFAULT_PORT: u16 = 5010;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedConnection {
    pub url: String,
    #[serde(default)]
    pub username: String,
    pub password: String,
    /// Reach the daemon through SSH rather than at `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl Origin {
    pub fn base_url(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }
}

#[derive(Debug, Clone)]
struct Active {
    saved: SavedConnection,
    origin: Origin,
    token: Option<String>,
    last_error: Option<String>,
}

static ACTIVE: RwLock<Option<Active>> = RwLock::new(None);
/// Held for the process lifetime: dropping it stops the forward.
static TUNNEL: std::sync::Mutex<Option<Tunnel>> = std::sync::Mutex::new(None);

impl Origin {
    fn loopback(port: u16) -> Origin {
        Origin {
            scheme: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port,
        }
    }
}

/// Starts the SSH forward and answers with the origin it serves, or why the session is down.
async fn open_tunnel(ssh: SshSettings) -> Result<(Tunnel, Origin), String> {
    let tunnel = Tunnel::start(ssh).await?;
    let origin = Origin::loopback(tunnel.local_port);
    Ok((tunnel, origin))
}

/// For `cmd_connect_remote`: tunnels in, logs in through the tunnel, and answers with the settings
/// to save, host key pinned. Nothing is kept running; the restart that follows opens its own.
pub async fn probe_ssh(
    ssh: SshSettings,
    username: &str,
    password: &str,
) -> Result<SshSettings, String> {
    let (tunnel, origin) = open_tunnel(ssh.clone()).await?;
    if let Some(e) = tunnel.first_error.clone() {
        return Err(e);
    }
    login(&origin, username, password).await.map_err(|e| {
        if e.starts_with("Could not reach") {
            format!(
                "SSH worked, but no Tendril server answered on port {} there. Is `tendril run` going on the server?",
                ssh.remote_port
            )
        } else {
            e
        }
    })?;
    Ok(SshSettings {
        host_key: tunnel.host_key.clone(),
        ..ssh
    })
}

/// Accepts what an operator is likely to type: `1.2.3.4`, `my-vps:5010`, `http://host:5010`, or a
/// tunnel's `https://name.trycloudflare.com`. Without a scheme it is `http` on port 5010; with one,
/// the scheme's own default port applies, which is what a tunnel or reverse proxy URL needs.
pub fn parse_origin(input: &str) -> Result<Origin, String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Enter the server's address".to_string());
    }
    let has_scheme = trimmed.contains("://");
    let with_scheme = if has_scheme {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("Invalid address: {e}"))?;
    let scheme = url.scheme().to_string();
    if scheme != "http" && scheme != "https" {
        return Err("The address must start with http:// or https://".to_string());
    }
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "The address has no host".to_string())?
        .to_string();
    let port = match url.port() {
        Some(port) => port,
        None if !has_scheme => DEFAULT_PORT,
        None => url.port_or_known_default().unwrap_or(DEFAULT_PORT),
    };
    Ok(Origin { scheme, host, port })
}

/// Logs in and answers with a session token, or a message fit to show the operator.
pub async fn login(origin: &Origin, username: &str, password: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct LoginResponse {
        token: String,
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let base = origin.base_url();
    let resp = client
        .post(format!("{base}/api/auth/login"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|e| format!("Could not reach a Tendril server at {base}: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        return resp
            .json::<LoginResponse>()
            .await
            .map(|r| r.token)
            .map_err(|_| format!("{base} answered, but not like a Tendril server"));
    }
    let server_message = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
    Err(match status.as_u16() {
        401 => "Wrong username or password".to_string(),
        429 => "Too many failed attempts; wait a minute and try again".to_string(),
        400 => format!(
            "{}. Set a password on the server first (TENDRIL_AUTH_PASSWORD, or Settings > Security & Tunneling there).",
            server_message.unwrap_or_else(|| "The server refused the login".to_string())
        ),
        _ => server_message.unwrap_or_else(|| format!("Login failed ({status})")),
    })
}

fn file_path(home: &Path) -> PathBuf {
    home.join(FILE_NAME)
}

pub fn load_saved(home: &Path) -> Option<SavedConnection> {
    let text = std::fs::read_to_string(file_path(home)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(home: &Path, saved: &SavedConnection) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let path = file_path(home);
    let text = serde_json::to_string_pretty(saved).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("Could not save the connection: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn clear(home: &Path) -> Result<(), String> {
    match std::fs::remove_file(file_path(home)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Could not remove the saved connection: {e}")),
    }
}

/// Activates the saved connection, if there is one. The first login is awaited (bounded) so the
/// window's first requests carry a token; after that a background task keeps it fresh.
pub fn init_at_startup(home: &Path) {
    let Some(saved) = load_saved(home) else {
        return;
    };
    let (origin, tunnel_error) = match &saved.ssh {
        Some(ssh) => match tauri::async_runtime::block_on(open_tunnel(ssh.clone())) {
            Ok((tunnel, origin)) => {
                let error = tunnel.first_error.clone();
                *TUNNEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tunnel);
                (origin, error)
            }
            Err(e) => {
                tracing::warn!("Ignoring saved SSH connection: {e}");
                return;
            }
        },
        None => match parse_origin(&saved.url) {
            Ok(origin) => (origin, None),
            Err(e) => {
                tracing::warn!("Ignoring saved remote connection: {e}");
                return;
            }
        },
    };
    let first = match tunnel_error {
        Some(e) => Err(e),
        None => tauri::async_runtime::block_on(async {
            tokio::time::timeout(
                Duration::from_secs(8),
                login(&origin, &saved.username, &saved.password),
            )
            .await
            .unwrap_or_else(|_| Err(format!("Timed out reaching {}", origin.base_url())))
        }),
    };
    let (token, last_error) = match first {
        Ok(token) => (Some(token), None),
        Err(e) => {
            tracing::warn!("Remote login failed: {e}");
            (None, Some(e))
        }
    };
    tracing::info!("Using remote Tendril server at {}", origin.base_url());
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(Active {
        saved,
        origin,
        token,
        last_error,
    });

    tauri::async_runtime::spawn(async {
        loop {
            let healthy = current().is_some_and(|a| a.token.is_some() && a.last_error.is_none());
            tokio::time::sleep(if healthy { REFRESH_EVERY } else { RETRY_EVERY }).await;
            let Some(active) = current() else { return };
            let result = login(
                &active.origin,
                &active.saved.username,
                &active.saved.password,
            )
            .await;
            let mut guard = ACTIVE.write().unwrap_or_else(|e| e.into_inner());
            if let Some(active) = guard.as_mut() {
                match result {
                    Ok(token) => {
                        active.token = Some(token);
                        active.last_error = None;
                    }
                    Err(e) => {
                        tracing::warn!("Remote login refresh failed: {e}");
                        // A token that has not expired yet is still worth sending; the next pass
                        // retries sooner because `healthy` requires the last attempt to have succeeded.
                        active.last_error = Some(e);
                    }
                }
            }
        }
    });
}

fn current() -> Option<Active> {
    ACTIVE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn is_active() -> bool {
    current().is_some()
}

/// What `read_master` answers while a remote connection is active. The pid is 0 because there is no
/// local process to watch; the status code paths branch on [`is_active`] before any pid check.
pub fn master_override() -> Option<MasterInfo> {
    let active = current()?;
    Some(MasterInfo {
        port: active.origin.port,
        pid: 0,
        secret: active.token.unwrap_or_default(),
        started_at: String::new(),
        host: active.origin.host,
        scheme: active.origin.scheme,
        version: String::new(),
        api_version: 1,
        capabilities: Vec::new(),
    })
}

/// For the webview: where the app is connected, never the password or token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatusDto {
    pub active: bool,
    pub url: Option<String>,
    pub username: Option<String>,
    pub authenticated: bool,
    pub error: Option<String>,
}

pub fn status() -> RemoteStatusDto {
    match current() {
        Some(active) => RemoteStatusDto {
            active: true,
            url: Some(match &active.saved.ssh {
                Some(ssh) => ssh.label(),
                None => active.origin.base_url(),
            }),
            username: Some(active.saved.username),
            authenticated: active.token.is_some(),
            error: active.last_error,
        },
        None => RemoteStatusDto {
            active: false,
            url: None,
            username: None,
            authenticated: false,
            error: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_or_ip_is_http_on_tendrils_port() {
        let o = parse_origin("203.0.113.7").unwrap();
        assert_eq!(o.base_url(), "http://203.0.113.7:5010");
        let o = parse_origin(" my-vps.example.com ").unwrap();
        assert_eq!(o.base_url(), "http://my-vps.example.com:5010");
    }

    #[test]
    fn an_explicit_port_or_scheme_is_kept() {
        assert_eq!(
            parse_origin("203.0.113.7:8080").unwrap().base_url(),
            "http://203.0.113.7:8080"
        );
        assert_eq!(
            parse_origin("https://abc.trycloudflare.com/")
                .unwrap()
                .base_url(),
            "https://abc.trycloudflare.com:443"
        );
        assert_eq!(
            parse_origin("http://host:5011").unwrap().base_url(),
            "http://host:5011"
        );
    }

    #[test]
    fn rejects_empty_and_other_schemes() {
        assert!(parse_origin("  ").is_err());
        assert!(parse_origin("ftp://host").is_err());
    }

    #[test]
    fn saves_loads_and_clears() {
        let home = std::env::temp_dir().join(format!("tendril-remote-{}", std::process::id()));
        let saved = SavedConnection {
            url: "1.2.3.4".into(),
            username: "admin".into(),
            password: "pw".into(),
            ssh: None,
        };
        save(&home, &saved).unwrap();
        assert_eq!(load_saved(&home), Some(saved));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file_path(&home))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        clear(&home).unwrap();
        assert_eq!(load_saved(&home), None);
        clear(&home).unwrap();
        std::fs::remove_dir_all(home).ok();
    }
}

/// Against a real daemon: `TENDRIL_REMOTE_TEST_URL=<addr> TENDRIL_REMOTE_TEST_PASSWORD=<pw>
/// cargo test -p tendril-app remote_live -- --ignored`. Logs in the way the app does, then drives the
/// ordinary `TendrilClient` with the resulting token.
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "needs a running daemon with a password set"]
    async fn remote_live_login_and_authenticated_calls() {
        let url = std::env::var("TENDRIL_REMOTE_TEST_URL").expect("TENDRIL_REMOTE_TEST_URL");
        let password =
            std::env::var("TENDRIL_REMOTE_TEST_PASSWORD").expect("TENDRIL_REMOTE_TEST_PASSWORD");
        let origin = parse_origin(&url).unwrap();

        assert_eq!(
            login(&origin, "", "definitely-wrong").await.unwrap_err(),
            "Wrong username or password"
        );
        let token = login(&origin, "", &password).await.unwrap();

        let client = crate::service::TendrilClient::new(origin.base_url(), Some(token));
        client.list_projects().await.unwrap();
        let listing = client.list_directories(None, false).await.unwrap();
        assert!(listing.get("entries").is_some());
    }
}
