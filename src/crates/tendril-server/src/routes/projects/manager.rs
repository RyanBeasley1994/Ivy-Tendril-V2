//! A project's Factory Manager and its runtime surroundings.
//!
//! * `POST /api/projects/:name/manager` finds or creates the project's manager chat session
//!   (see `tendril_core::chat::manager_brief`) and returns it. The conversation itself then runs
//!   through the ordinary chat routes.
//! * `GET /api/projects/:name/docker` lists the Docker containers that belong to the project.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;
use tendril_core::chat::manager_brief::{manager_briefing, manager_session_id};
use tendril_core::chat::ChatMessage;
use tendril_core::config::{expand_variables, load_config};
use uuid::Uuid;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

struct ProjectInfo {
    name: String,
    repos: Vec<String>,
    context: String,
}

fn find_project(state: &AppState, name: &str) -> Result<ProjectInfo, Response> {
    let settings = load_config(&state.config_path).map_err(|e| {
        error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load config: {e}"))
    })?;
    let home = state.tendril_home.to_string_lossy().to_string();
    settings
        .projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| ProjectInfo {
            name: p.name.clone(),
            repos: p.repos.iter().map(|r| expand_variables(&r.path, &home)).collect(),
            context: p.context.clone(),
        })
        .ok_or_else(|| error(StatusCode::NOT_FOUND, format!("Project '{name}' not found")))
}

pub async fn get_or_create_project_manager(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let project = match find_project(&state, &name) {
        Ok(p) => p,
        Err(response) => return response,
    };
    let id = manager_session_id(&project.name);
    let title = format!("Manager · {}", project.name);
    match state.chat_manager.get_or_create_session_with_id(&id, &title).await {
        Ok((session, created)) => {
            if created {
                let briefing = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "system".to_string(),
                    content: manager_briefing(&project.name, &project.repos, &project.context),
                    timestamp: Utc::now(),
                    agent_id: None,
                    model_id: None,
                    raw_stream: None,
                    effort: None,
                };
                if let Err(e) = state.chat_manager.add_message(&id, briefing).await {
                    return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                }
            }
            let session = state.chat_manager.get_session(&id).await.unwrap_or(session);
            (StatusCode::OK, Json(json!(session))).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `docker ps -a`, narrowed to what belongs to the project: a compose project whose working
/// directory is inside one of its repos, or a container whose name carries the project's name.
pub async fn project_docker(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let project = match find_project(&state, &name) {
        Ok(p) => p,
        Err(response) => return response,
    };
    let output = tokio::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{json .}}"])
        .output()
        .await;
    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            return Json(json!({
                "available": false,
                "reason": String::from_utf8_lossy(&o.stderr).trim().to_string(),
                "containers": [],
            }))
            .into_response()
        }
        Err(_) => {
            return Json(json!({
                "available": false,
                "reason": "Docker is not installed or not on PATH",
                "containers": [],
            }))
            .into_response()
        }
    };

    let needle = project.name.to_ascii_lowercase();
    let containers: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|c| {
            let labels = c["Labels"].as_str().unwrap_or("");
            let in_repo = project.repos.iter().any(|repo| {
                labels.contains(&format!("com.docker.compose.project.working_dir={repo}"))
            });
            in_repo
                || c["Names"]
                    .as_str()
                    .map(|n| n.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
        })
        .map(|c| {
            json!({
                "id": c["ID"],
                "name": c["Names"],
                "image": c["Image"],
                "state": c["State"],
                "status": c["Status"],
                "ports": c["Ports"],
            })
        })
        .collect();
    Json(json!({ "available": true, "containers": containers })).into_response()
}
