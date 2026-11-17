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
pub async fn cmd_get_global_engine() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.get_global_engine().await
}

#[tauri::command]
pub async fn cmd_set_global_engine(body: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.set_global_engine(body).await
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
pub async fn cmd_telegram(action: String, token: Option<String>) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.telegram(&action, token).await
}

/// Writes a file the operator chose to save (a support ticket's attachment) where they picked in the
/// save dialog.
#[tauri::command]
pub async fn cmd_save_file(path: String, data_base64: String) -> Result<(), BridgeError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|e| BridgeError::new("SAVE_FAILED", format!("The file's data was unreadable: {e}")))?;
    std::fs::write(&path, bytes).map_err(|e| BridgeError::new("SAVE_FAILED", format!("Could not save to {path}: {e}")))
}

/// A project's extra manager tabs: `list`, `open` a new one, or `close` / `reopen` / `rename` one.
#[tauri::command]
pub async fn cmd_manager_tabs(
    project_name: String,
    action: String,
    tab_id: Option<String>,
    title: Option<String>,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.manager_tabs(&project_name, &action, tab_id.as_deref(), title).await
}

#[tauri::command]
pub async fn cmd_list_manager_tasks(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .list_manager_tasks(&project_name)
        .await
}

#[tauri::command]
pub async fn cmd_project_my_prs(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.project_my_prs(&project_name).await
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

/// The Support page's one command: `path` must be under `/api/support/`.
#[tauri::command]
pub async fn cmd_support(
    method: String,
    path: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .support_request(&method, &path, body)
        .await
}

/// Signing an agent's CLI back in: `GET`/`POST`/`DELETE` on its login session, or `POST` with `code`
/// to hand Claude the code from its callback page.
#[tauri::command]
pub async fn cmd_agent_login(
    method: String,
    agent: String,
    code: Option<String>,
) -> Result<serde_json::Value, BridgeError> {
    let body = code.as_ref().map(|code| json!({ "code": code }));
    get_client_from_master()?
        .agent_login_request(&method, &agent, code.is_some(), body)
        .await
}

/// The main and fallback Claude subscriptions; `remove_fallback` signs the fallback out.
#[tauri::command]
pub async fn cmd_claude_accounts(remove_fallback: Option<bool>) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .claude_accounts(remove_fallback.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn cmd_list_project_links(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .list_project_links(&project_name)
        .await
}

/// `link` is the daemon's `LinkInput` as-is: url, label, username, password, notes, optional slug.
#[tauri::command]
pub async fn cmd_set_project_link(
    project_name: String,
    link: serde_json::Value,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .set_project_link(&project_name, link)
        .await
}

#[tauri::command]
pub async fn cmd_delete_project_link(
    project_name: String,
    slug: String,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .delete_project_link(&project_name, &slug)
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

#[tauri::command]
pub async fn cmd_project_manager_activity(project_name: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.project_manager_activity(&project_name).await
}
