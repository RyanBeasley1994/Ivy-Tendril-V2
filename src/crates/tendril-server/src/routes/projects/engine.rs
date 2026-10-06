//! A project's engine: the coding agent and model its manager and missions run on.
//!
//! Switching it (say, from Claude to a local model when Claude is rate-limited) is one call that
//! changes everything in the project at once: the manager's own next turns, every live mission's
//! role agents from their next job, and the guidance the manager follows for anything new. The job
//! running at the moment of the switch keeps the agent it started on.
//!
//! * `GET  /api/projects/:name/engine`
//! * `PUT  /api/projects/:name/engine`  body `{ "agent", "model"?, "effort"? }`

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tendril_core::chat::manager_brief::manager_session_id;
use tendril_core::chat::ChatMessage;
use tendril_core::config::{get_project_root_dir, load_config};
use tendril_core::missions::model::{MissionAgents, RoleAgent};
use tendril_core::missions::service;
use tendril_core::missions::store::list_missions;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engine {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

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

fn engine_file(state: &AppState, project: &str) -> std::path::PathBuf {
    get_project_root_dir(&state.tendril_home, project).join("engine.json")
}

fn read_engine(state: &AppState, project: &str) -> Option<Engine> {
    std::fs::read_to_string(engine_file(state, project))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub async fn get_project_engine(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let project = match project_name(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    Json(json!({ "engine": read_engine(&state, &project) })).into_response()
}

pub async fn set_project_engine(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(engine): Json<Engine>,
) -> impl IntoResponse {
    let project = match project_name(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if engine.agent.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "An agent is required");
    }

    let file = engine_file(&state, &project);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&file, serde_json::to_vec_pretty(&engine).unwrap_or_default()) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }

    let role = RoleAgent {
        agent: engine.agent.clone(),
        model: engine.model.clone(),
        effort: engine.effort.clone(),
    };

    // Every live mission in the project, from its next job.
    let mut updated = 0usize;
    for file in list_missions(&state.mission_driver.paths().missions_dir) {
        if !file.mission.project.eq_ignore_ascii_case(&project) || file.mission.state.is_terminal() {
            continue;
        }
        let agents = MissionAgents {
            planner: Some(role.clone()),
            worker: Some(role.clone()),
            judge: Some(role.clone()),
            validator: Some(role.clone()),
        };
        if service::set_agents(std::path::Path::new(&file.folder_path), agents).is_ok() {
            updated += 1;
        }
    }

    // The manager itself, and a note it reads on its next turn so new work uses the same engine.
    let manager_id = manager_session_id(&project);
    let mut manager_updated = false;
    if state
        .chat_manager
        .set_session_agent(
            &manager_id,
            &engine.agent,
            engine.model.as_deref(),
            engine.effort.as_deref(),
        )
        .await
        .is_ok()
    {
        manager_updated = true;
        let harness = role.describe();
        let spec = {
            let mut s = engine.agent.clone();
            if let Some(m) = &engine.model {
                s.push(':');
                s.push_str(m);
            }
            s
        };
        let note = format!(
            "Engine switched by the operator: this project now runs on {harness}. Live missions \
             were moved to it from their next job. For anything new, pass \
             `--planner {spec} --worker {spec} --judge {spec} --validator {spec}` to \
             `tendril mission create`, and start plan jobs as usual (they use the configured \
             default agent unless told otherwise). Do not switch back unless the operator asks."
        );
        let _ = state
            .chat_manager
            .add_message(
                &manager_id,
                ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "system".to_string(),
                    content: note,
                    timestamp: Utc::now(),
                    agent_id: None,
                    model_id: None,
                    raw_stream: None,
                    effort: None,
                },
            )
            .await;
    }

    Json(json!({
        "engine": engine,
        "missionsUpdated": updated,
        "managerUpdated": manager_updated,
    }))
    .into_response()
}
