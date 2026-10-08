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
use tendril_core::agents::project_engine::{apply_mission_roles, GlobalEngine, ProjectEngine, ROLES};
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

// ---- the global engine ---------------------------------------------------------------------------

pub async fn get_global_engine(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let engine = GlobalEngine::load(&state.tendril_home);
    Json(json!({ "roles": engine.roles, "fallbacks": engine.fallbacks })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct SetGlobalEngineRequest {
    /// A role name to its new engine, or `null` to put that role back on the configured default.
    #[serde(default)]
    pub roles: BTreeMap<String, Option<RoleAgent>>,
    /// The whole fallback chain, in order, replacing the current one. Omit to leave it alone.
    #[serde(default)]
    pub fallbacks: Option<Vec<RoleAgent>>,
}

/// `PUT /api/engine`: sets the engine for every project that does not override it, and the rate-limit
/// fallback chain. Live missions and managers of projects without their own setting follow at once.
pub async fn set_global_engine(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetGlobalEngineRequest>,
) -> impl IntoResponse {
    if let Some(bad) = body.roles.keys().find(|k| !ROLES.contains(&k.as_str())) {
        return error(StatusCode::BAD_REQUEST, format!("'{bad}' is not a role; the roles are {}", ROLES.join(", ")));
    }
    let mut engine = GlobalEngine::load(&state.tendril_home);
    for (role, value) in &body.roles {
        engine.set(role, value.clone());
    }
    if let Some(chain) = body.fallbacks.clone() {
        engine.set_fallbacks(chain);
    }
    if let Err(e) = engine.save(&state.tendril_home) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }

    let changed: Vec<&str> = body.roles.keys().map(String::as_str).collect();
    let mission_roles: Vec<&str> = changed.iter().copied().filter(|r| *r != "manager").collect();
    let settings = load_config(&state.config_path).unwrap_or_default();
    let mut missions_updated = 0usize;
    let mut managers_updated = 0usize;
    for project in settings.projects.iter().map(|p| p.name.clone()) {
        // A role the project sets itself is not touched by a change to the global one.
        let own = ProjectEngine::load(&state.tendril_home, &project);
        let inherited: Vec<&str> = changed.iter().copied().filter(|r| own.role(r).is_none()).collect();
        if inherited.is_empty() {
            continue;
        }
        let effective = ProjectEngine::load_effective(&state.tendril_home, &project);
        let live_roles: Vec<&str> = mission_roles.iter().copied().filter(|r| inherited.contains(r)).collect();
        if !live_roles.is_empty() {
            for file in list_missions(&state.mission_driver.paths().missions_dir) {
                if !file.mission.project.eq_ignore_ascii_case(&project) || file.mission.state.is_terminal() {
                    continue;
                }
                let merged = apply_mission_roles(file.mission.agents.clone(), &live_roles, &effective);
                if service::set_agents(std::path::Path::new(&file.folder_path), merged).is_ok() {
                    missions_updated += 1;
                }
            }
        }
        if inherited.contains(&"manager") {
            let (agent, model, effort) = match effective.role("manager") {
                Some(r) => (r.agent.clone(), r.model.clone(), r.effort.clone()),
                None => (state.settings_snapshot().settings.coding_agent.clone(), None, None),
            };
            if state
                .chat_manager
                .set_session_agent(&manager_session_id(&project), &agent, model.as_deref(), effort.as_deref())
                .await
                .is_ok()
            {
                managers_updated += 1;
            }
        }
    }
    Json(json!({
        "roles": engine.roles,
        "fallbacks": engine.fallbacks,
        "missionsUpdated": missions_updated,
        "managersUpdated": managers_updated,
    }))
    .into_response()
}
