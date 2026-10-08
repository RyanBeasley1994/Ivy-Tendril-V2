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
use tendril_core::chat::manager_brief::{is_briefing, manager_briefing, manager_session_id};
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

/// The operator's own standing orders for a project's manager, from
/// `<TENDRIL_HOME>/Projects/<Project>/manager-policy.md`, when they have written one.
fn read_policy(state: &AppState, project: &str) -> Option<String> {
    std::fs::read_to_string(
        tendril_core::config::get_project_root_dir(&state.tendril_home, project)
            .join("manager-policy.md"),
    )
    .ok()
}

/// Puts the current briefing in place of the one a manager already has, when they differ: the briefing
/// itself changed, or the operator edited `manager-policy.md`, the project's context or its repositories.
/// A manager with no session, or a session with no briefing in it, is left alone, and so is one in the
/// middle of a turn: that turn is writing the session, and it already has the briefing it started with.
pub(crate) async fn refresh_briefing(state: &AppState, project: &str) {
    let Ok(project) = find_project(state, project) else { return };
    let id = manager_session_id(&project.name);
    if state.chat_manager.is_generating(&id).await {
        return;
    }
    let Ok(session) = state.chat_manager.get_session(&id).await else { return };
    let Some(old) = session.messages.iter().find(|m| m.role == "system" && is_briefing(&m.content)) else {
        return;
    };
    let fresh = manager_briefing(
        &project.name,
        &project.repos,
        &project.context,
        read_policy(state, &project.name).as_deref(),
    );
    if old.content != fresh {
        let _ = state.chat_manager.replace_message_content(&id, &old.id, &fresh).await;
    }
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
                if let Some(r) = tendril_core::agents::project_engine::ProjectEngine::load_effective(&state.tendril_home, &project.name).role("manager") {
                    let _ = state.chat_manager.set_session_agent(&id, &r.agent, r.model.as_deref(), r.effort.as_deref()).await;
                }
                let briefing = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "system".to_string(),
                    content: manager_briefing(
                        &project.name,
                        &project.repos,
                        &project.context,
                        read_policy(&state, &project.name).as_deref(),
                    ),
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
            if !created {
                refresh_briefing(&state, &project.name).await;
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
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            // The common case on a hosted daemon: the CLI is there but the socket is not.
            let reason = if stderr.contains("docker.sock") || stderr.contains("Cannot connect") {
                "The daemon cannot reach Docker. It is probably running inside a container with no \
                 Docker socket; mount /var/run/docker.sock into it to list containers here."
                    .to_string()
            } else {
                stderr
            };
            return Json(json!({ "available": false, "reason": reason, "containers": [] })).into_response();
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

/// `GET /api/projects/owners` — project name to the owner of its first repo's `origin` remote
/// (`null` when the repo is local-only or has no remote).
pub async fn project_owners(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = load_config(&state.config_path).unwrap_or_default();
    let home = state.tendril_home.to_string_lossy().to_string();
    let mut owners = serde_json::Map::new();
    for project in &settings.projects {
        let owner = match project.repos.first() {
            Some(repo) => {
                let path = expand_variables(&repo.path, &home);
                tokio::process::Command::new("git")
                    .args(["-C", &path, "remote", "get-url", "origin"])
                    .output()
                    .await
                    .ok()
                    .filter(|o| o.status.success())
                    .and_then(|o| tendril_core::git::workspace::remote_owner(&String::from_utf8_lossy(&o.stdout)))
            }
            None => None,
        };
        owners.insert(project.name.clone(), json!(owner));
    }
    Json(Value::Object(owners))
}


/// `GET /api/projects/managers` — whether each project's manager is in the middle of a turn, and when
/// its conversation last moved, and how many things wait on the operator (`needsYou`). A manager that
/// has never been opened is simply not working.
pub async fn managers_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = load_config(&state.config_path).unwrap_or_default();
    let mut out = serde_json::Map::new();
    for project in &settings.projects {
        let id = manager_session_id(&project.name);
        let working = state.chat_manager.is_generating(&id).await;
        let updated = state
            .chat_manager
            .get_session(&id)
            .await
            .ok()
            .map(|s| s.updated_at.to_rfc3339());
        let attention = crate::attention::for_project(&state, &project.name).await;
        out.insert(
            project.name.clone(),
            json!({
                "working": working,
                "updatedAt": updated,
                // What waits on the operator: missions to approve or resume, and a question asked.
                "needsYou": attention.count(),
                "waitingMissions": attention.waiting_missions,
                "asked": attention.asked,
            }),
        );
    }
    Json(Value::Object(out))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeRequest {
    pub after_seconds: u64,
    #[serde(default)]
    pub note: String,
}

/// `POST /api/projects/:name/manager/wake` — the manager asks to be woken later, with a note.
pub async fn schedule_manager_wake(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<WakeRequest>,
) -> impl IntoResponse {
    let project = match find_project(&state, &name) {
        Ok(p) => p,
        Err(response) => return response,
    };
    match crate::manager_scheduler::add_wake(&state.tendril_home, &project.name, body.after_seconds, &body.note).await {
        Ok(wake) => (StatusCode::OK, Json(json!(wake))).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct WatchPrRequest {
    pub pr: u64,
    #[serde(default)]
    pub repo: Option<String>,
}

/// `POST /api/projects/:name/manager/watch` — the manager asks the daemon to watch a pull request's
/// checks and wake it with the result once they have all finished.
pub async fn watch_pull_request(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<WatchPrRequest>,
) -> impl IntoResponse {
    let project = match find_project(&state, &name) {
        Ok(p) => p,
        Err(response) => return response,
    };
    let watch = crate::pr_watch::add_watch(&state.tendril_home, &project.name, body.pr, body.repo).await;
    (StatusCode::OK, Json(json!(watch))).into_response()
}
