//! A project's engines: the coding agent and model each role runs on, set one role at a time.
//!
//! * `GET /api/projects/:name/engine`  -> `{ roles: { worker: { agent, model?, effort? }, ... } }`
//! * `PUT /api/projects/:name/engine`  body `{ roles: { worker: { agent, model?, effort? } | null, ... } }`
//!   sets each role it names and clears the ones sent as `null`; roles it does not name are untouched.
//!
//! A change takes effect everywhere in the project at once, and can be made at any time (say, moving a
//! project to a local model while Claude is rate-limited): the manager's own next turns, every live
//! mission's matching roles from their next job, new missions, and new plan jobs. A job already
//! running keeps the agent it started on.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use tendril_core::agents::project_engine::{apply_mission_roles, ProjectEngine, ROLES};
use tendril_core::chat::manager_brief::manager_session_id;
use tendril_core::chat::ChatMessage;
use tendril_core::config::load_config;
use tendril_core::missions::model::RoleAgent;
use tendril_core::missions::service;
use tendril_core::missions::store::list_missions;
use uuid::Uuid;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn project_name(state: &AppState, name: &str) -> Result<String, Response> {
    let settings = load_config(&state.config_path).map_err(|e| {
        error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load config: {e}"))
    })?;
    settings
        .projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| p.name.clone())
        .ok_or_else(|| error(StatusCode::NOT_FOUND, format!("Project '{name}' not found")))
}

pub async fn get_project_engine(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let project = match project_name(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let engine = ProjectEngine::load(&state.tendril_home, &project);
    Json(json!({ "roles": engine.roles })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct SetEngineRequest {
    /// A role name to its new engine, or `null` to put that role back on the default.
    pub roles: BTreeMap<String, Option<RoleAgent>>,
}

pub async fn set_project_engine(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<SetEngineRequest>,
) -> impl IntoResponse {
    let project = match project_name(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Some(bad) = body.roles.keys().find(|k| !ROLES.contains(&k.as_str())) {
        return error(
            StatusCode::BAD_REQUEST,
            format!("'{bad}' is not a role; the roles are {}", ROLES.join(", ")),
        );
    }

    let mut engine = ProjectEngine::load(&state.tendril_home, &project);
    for (role, value) in &body.roles {
        engine.set(role, value.clone());
    }
    if let Err(e) = engine.save(&state.tendril_home, &project) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }

    let changed: Vec<&str> = body.roles.keys().map(String::as_str).collect();
    let mission_roles: Vec<&str> = changed.iter().copied().filter(|r| *r != "manager").collect();

    // Every live mission in the project follows, from its next job.
    let mut missions_updated = 0usize;
    if !mission_roles.is_empty() {
        for file in list_missions(&state.mission_driver.paths().missions_dir) {
            if !file.mission.project.eq_ignore_ascii_case(&project) || file.mission.state.is_terminal() {
                continue;
            }
            let merged = apply_mission_roles(file.mission.agents.clone(), &mission_roles, &engine);
            if service::set_agents(std::path::Path::new(&file.folder_path), merged).is_ok() {
                missions_updated += 1;
            }
        }
    }

    // The manager's own chat, if its role changed. Cleared means "the configured default agent".
    let mut manager_updated = false;
    if changed.contains(&"manager") {
        let manager_id = manager_session_id(&project);
        let (agent, model, effort) = match engine.role("manager") {
            Some(r) => (r.agent.clone(), r.model.clone(), r.effort.clone()),
            None => (state.settings_snapshot().settings.coding_agent.clone(), None, None),
        };
        if state
            .chat_manager
            .set_session_agent(&manager_id, &agent, model.as_deref(), effort.as_deref())
            .await
            .is_ok()
        {
            manager_updated = true;
        }
    }

    // A short note the manager reads, so it knows why things are running where they are.
    let summary: Vec<String> = changed
        .iter()
        .map(|r| match engine.role(r) {
            Some(v) => format!("{r}: {}", v.describe()),
            None => format!("{r}: default"),
        })
        .collect();
    let _ = state
        .chat_manager
        .add_message(
            &manager_session_id(&project),
            ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: "system".to_string(),
                content: format!(
                    "Engine switched by the operator ({}). Live missions follow from their next job, and \
                     new missions and plan jobs pick it up automatically. You do not need to pass agent \
                     flags.",
                    summary.join("; ")
                ),
                timestamp: Utc::now(),
                agent_id: None,
                model_id: None,
                raw_stream: None,
                effort: None,
            },
        )
        .await;

    Json(json!({
        "roles": engine.roles,
        "missionsUpdated": missions_updated,
        "managerUpdated": manager_updated,
    }))
    .into_response()
}
