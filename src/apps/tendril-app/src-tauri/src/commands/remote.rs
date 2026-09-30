//! Connecting the desktop app to a Tendril server on another machine, over SSH. See
//! `service::remote` and `service::ssh_tunnel`.

use crate::daemon::resolve_tendril_home;
use crate::error::BridgeError;
use crate::service::remote::{self, RemoteStatusDto, SavedConnection};
use crate::service::ssh_tunnel::{self, SshSettings};

/// Which server the app is using. Carries the address and username, never the password or token.
#[tauri::command]
pub async fn cmd_get_remote_connection() -> Result<RemoteStatusDto, BridgeError> {
    Ok(remote::status())
}

/// Opens an SSH tunnel to the server and logs in to Tendril through it; on success saves the
/// connection (host key pinned) and restarts the app onto it.
///
/// Both the SSH login and the Tendril login happen before anything is saved, so a wrong password or
/// an unreachable host is reported here and leaves the current connection untouched.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn cmd_connect_remote(
    app: tauri::AppHandle,
    ssh_address: String,
    ssh_username: Option<String>,
    ssh_password: String,
    remote_port: Option<u16>,
    username: Option<String>,
    password: String,
) -> Result<(), BridgeError> {
    let (host, port, ssh_username) =
        ssh_tunnel::parse_address(&ssh_address, ssh_username.as_deref().unwrap_or_default())
            .map_err(|e| BridgeError::new("INVALID_REMOTE_URL", e))?;
    let ssh = SshSettings {
        host,
        port,
        username: ssh_username,
        password: ssh_password,
        remote_port: remote_port.unwrap_or(remote::DEFAULT_PORT),
        host_key: None,
    };
    let username = username.unwrap_or_default().trim().to_string();
    let ssh = remote::probe_ssh(ssh, &username, &password)
        .await
        .map_err(|e| BridgeError::new("REMOTE_LOGIN_FAILED", e))?;
    remote::save(
        &resolve_tendril_home(),
        &SavedConnection {
            url: ssh.label(),
            username,
            password,
            ssh: Some(ssh),
        },
    )
    .map_err(|e| BridgeError::new("REMOTE_SAVE_FAILED", e))?;
    restart_soon(app);
    Ok(())
}

/// Forgets the saved server and restarts the app back onto the local one.
#[tauri::command]
pub async fn cmd_disconnect_remote(app: tauri::AppHandle) -> Result<(), BridgeError> {
    remote::clear(&resolve_tendril_home())
        .map_err(|e| BridgeError::new("REMOTE_SAVE_FAILED", e))?;
    restart_soon(app);
    Ok(())
}

/// The bridges are wired once at startup, so switching servers is a restart. Deferred so the
/// command can answer first and the webview is not torn down mid-call.
fn restart_soon(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        app.restart();
    });
}
