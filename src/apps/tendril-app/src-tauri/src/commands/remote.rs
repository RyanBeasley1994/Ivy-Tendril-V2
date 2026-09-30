//! Connecting the desktop app to a Tendril server on another machine. See `service::remote`.

use crate::daemon::resolve_tendril_home;
use crate::error::BridgeError;
use crate::service::remote::{self, RemoteStatusDto, SavedConnection};

/// Which server the app is using. Carries the address and username, never the password or token.
#[tauri::command]
pub async fn cmd_get_remote_connection() -> Result<RemoteStatusDto, BridgeError> {
    Ok(remote::status())
}

/// Logs in to the server at `url`; on success saves the connection and restarts the app onto it.
///
/// The login happens before anything is saved, so a wrong password or an unreachable address is
/// reported here and leaves the current connection untouched.
#[tauri::command]
pub async fn cmd_connect_remote(
    app: tauri::AppHandle,
    url: String,
    username: Option<String>,
    password: String,
) -> Result<(), BridgeError> {
    let origin =
        remote::parse_origin(&url).map_err(|e| BridgeError::new("INVALID_REMOTE_URL", e))?;
    let username = username.unwrap_or_default().trim().to_string();
    remote::login(&origin, &username, &password)
        .await
        .map_err(|e| BridgeError::new("REMOTE_LOGIN_FAILED", e))?;
    remote::save(
        &resolve_tendril_home(),
        &SavedConnection {
            url: origin.base_url(),
            username,
            password,
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
