//! Bridge commands for missions. Thin wrappers over `/api/missions`: the daemon's driver owns every
//! transition, so there is nothing to decide here.

use crate::commands::get_client_from_master;
use crate::error::BridgeError;

#[tauri::command]
pub async fn cmd_list_missions() -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.list_missions().await
}

#[tauri::command]
pub async fn cmd_get_mission(id: String) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.get_mission(&id).await
}

/// `request` is `{ title, goal, project, maxAttempts?, maxReplans?, maxCost? }`.
#[tauri::command]
pub async fn cmd_create_mission(
    request: serde_json::Value,
    target: Option<String>,
) -> Result<serde_json::Value, BridgeError> {
    crate::commands::get_client_for_target(target.as_deref())?
        .create_mission(&request)
        .await
}

/// `action` is one of approve, pause, resume, cancel or reconcile.
#[tauri::command]
pub async fn cmd_mission_action(
    id: String,
    action: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .mission_action(&id, &action, body.as_ref())
        .await
}

/// `budget` is `{ maxAttempts?, maxReplans?, maxCost? }`.
#[tauri::command]
pub async fn cmd_set_mission_budget(
    id: String,
    budget: serde_json::Value,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.put_mission(&id, "budget", &budget).await
}

/// `milestones` is a list of `{ title, objective, spec, acceptance[] }`; replaces them all, so only
/// before approval.
#[tauri::command]
pub async fn cmd_set_mission_milestones(
    id: String,
    milestones: serde_json::Value,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?
        .put_mission(&id, "milestones", &serde_json::json!({ "milestones": milestones }))
        .await
}

/// `agents` is `{ planner?, worker?, judge?, validator? }`, each `{ agent, model?, effort? }`.
#[tauri::command]
pub async fn cmd_set_mission_agents(
    id: String,
    agents: serde_json::Value,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.put_mission(&id, "agents", &agents).await
}

/// The branch names `git` (`{ branchPrefix?, branchTemplate?, missionBranchTemplate? }`) would give.
#[tauri::command]
pub async fn cmd_preview_branch_names(
    git: serde_json::Value,
) -> Result<serde_json::Value, BridgeError> {
    get_client_from_master()?.preview_branch_names(&git).await
}
