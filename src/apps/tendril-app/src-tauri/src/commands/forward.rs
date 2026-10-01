//! Port forwards to the daemon's host. See `service::port_forward`.

use crate::error::BridgeError;
use crate::service::port_forward::{self, PortForward};

/// Opens `port` from the server on this machine; answers where it landed.
#[tauri::command]
pub async fn cmd_forward_port(port: u16) -> Result<PortForward, BridgeError> {
    port_forward::forward(port)
        .await
        .map_err(|e| BridgeError::new("FORWARD_FAILED", e))
}

#[tauri::command]
pub async fn cmd_list_port_forwards() -> Result<Vec<PortForward>, BridgeError> {
    Ok(port_forward::list())
}

#[tauri::command]
pub async fn cmd_stop_port_forward(port: u16) -> Result<bool, BridgeError> {
    Ok(port_forward::stop(port))
}
