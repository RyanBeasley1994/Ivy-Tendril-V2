//! Bridge commands for the Git page. Thin wrappers over `/api/git/**`: the daemon decides what a
//! repository is, what may be run on it, and in what order, so there is nothing to decide here. The
//! webview names a repository only by the id the daemon gave it.

use super::get_client_from_master;
use crate::error::BridgeError;

#[tauri::command]
pub async fn cmd_git_repos() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_repos().await
}

#[tauri::command]
pub async fn cmd_git_prs() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_prs().await
}

#[tauri::command]
pub async fn cmd_git_repo(id: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_repo(&id).await
}

#[tauri::command]
pub async fn cmd_git_graph(id: String, limit: u32, branch: Option<String>) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_graph(&id, limit, branch.as_deref()).await
}

#[tauri::command]
pub async fn cmd_git_commit(id: String, hash: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_commit(&id, &hash).await
}

/// `request` is `{ target: { kind: "commit"|"staged"|"unstaged"|"untracked", hash? }, path, origPath? }`.
#[tauri::command]
pub async fn cmd_git_diff(id: String, request: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_diff(&id, request).await
}

#[tauri::command]
pub async fn cmd_git_history(id: String, path: String, limit: u32) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_history(&id, &path, limit).await
}

/// `op` is one of the daemon's `RepoOp` variants, tagged by `op`.
#[tauri::command]
pub async fn cmd_git_op(id: String, op: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_op(&id, op).await
}

#[tauri::command]
pub async fn cmd_git_pr_prefill(id: String, head: String, base: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_pr_prefill(&id, &head, &base).await
}

/// `request` is `{ head, base, title, body, draft }`.
#[tauri::command]
pub async fn cmd_git_create_pr(id: String, request: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.git_create_pr(&id, request).await
}
