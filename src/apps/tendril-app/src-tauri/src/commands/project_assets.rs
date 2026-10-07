//! Project memory files and repo-asset import — thin proxies over the daemon's
//! `/api/projects/:name/memory` and `/api/projects/:name/repo-assets/*` routes, for the same reason
//! every command here exists: the daemon's bearer secret stays on the native side.
//!
//! No `tracing` line on the repo-asset calls: `source` can be a URL with a token in it.

use super::get_client_from_master;
use crate::error::BridgeError;
use serde_json::json;

#[tauri::command]
pub async fn cmd_list_api_keys() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.list_api_keys().await
}

// No `tracing` line: the response carries the key itself.
#[tauri::command]
pub async fn cmd_create_api_key(name: String, write: bool) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.create_api_key(&name, write).await
}

#[tauri::command]
pub async fn cmd_revoke_api_key(id: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.revoke_api_key(&id).await
}

#[tauri::command]
pub async fn cmd_mission_evidence(id: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.mission_evidence(&id).await
}

#[tauri::command]
pub async fn cmd_project_evidence(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.project_evidence(&project_name).await
}

#[tauri::command]
pub async fn cmd_get_project_engine(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.get_project_engine(&project_name).await
}

#[tauri::command]
pub async fn cmd_set_project_engine(
    project_name: String,
    roles: serde_json::Value,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .set_project_engine(&project_name, roles)
        .await
}

#[tauri::command]
pub async fn cmd_managers_status() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.managers_status().await
}

#[tauri::command]
pub async fn cmd_project_owners() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.project_owners().await
}

#[tauri::command]
pub async fn cmd_get_project_manager(
    project_name: String,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .get_project_manager(&project_name)
        .await
}

#[tauri::command]
pub async fn cmd_project_docker(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .project_docker(&project_name)
        .await
}

#[tauri::command]
pub async fn cmd_list_project_memory(
    project_name: String,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .list_project_memory(&project_name)
        .await
}

#[tauri::command]
pub async fn cmd_get_project_memory(
    project_name: String,
    file_name: String,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .get_project_memory(&project_name, &file_name)
        .await
}

#[tauri::command]
pub async fn cmd_put_project_memory(
    project_name: String,
    file_name: String,
    content: String,
    previous_file_name: Option<String>,
) -> Result<serde_json::Value, BridgeError> {
    if file_name.trim().is_empty() {
        return Err(BridgeError::validation("A memory file name is required"));
    }
    get_client_from_master()?
        .put_project_memory(
            &project_name,
            file_name.trim(),
            json!({ "content": content, "previousFileName": previous_file_name }),
        )
        .await
}

#[tauri::command]
pub async fn cmd_delete_project_memory(
    project_name: String,
    file_name: String,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .delete_project_memory(&project_name, &file_name)
        .await
}

/// `kind` is `skills` or `mcpServers`; the daemon rejects anything else.
#[tauri::command]
pub async fn cmd_scan_repo_assets(
    project_name: String,
    kind: String,
    source: String,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .scan_repo_assets(&project_name, json!({ "kind": kind, "source": source }))
        .await
}

#[tauri::command]
pub async fn cmd_import_repo_assets(
    project_name: String,
    kind: String,
    source: String,
    names: Vec<String>,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .import_repo_assets(
            &project_name,
            json!({ "kind": kind, "source": source, "names": names }),
        )
        .await
}
