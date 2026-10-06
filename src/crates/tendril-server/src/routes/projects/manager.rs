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
use tendril_core::chat::manager_brief::{
    briefing_is_current, is_briefing, manager_briefing, manager_session_id,
};
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
            if !created {
                // A manager opened before the briefing changed gets the current one in place.
                if let Some(old) = session
                    .messages
                    .iter()
                    .find(|m| m.role == "system" && is_briefing(&m.content))
                {
                    if !briefing_is_current(&old.content) {
                        let fresh =
                            manager_briefing(&project.name, &project.repos, &project.context);
                        let _ = state
                            .chat_manager
                            .replace_message_content(&id, &old.id, &fresh)
                            .await;
                    }
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

/// The owner segment of a git remote: `github.com/<owner>/<repo>` in https, ssh and scp forms.
fn remote_owner(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((_, rest)) = url.split_once(':') {
        rest
    } else {
        return None;
    };
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let owner = segments.next()?;
    // `owner/repo` needs a repo after it; a lone segment is not an owner.
    segments.next()?;
    Some(owner.to_string())
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
                    .and_then(|o| remote_owner(&String::from_utf8_lossy(&o.stdout)))
            }
            None => None,
        };
        owners.insert(project.name.clone(), json!(owner));
    }
    Json(Value::Object(owners))
}

#[cfg(test)]
mod owner_tests {
    use super::remote_owner;

    #[test]
    fn parses_common_remote_forms() {
        assert_eq!(remote_owner("https://github.com/acme/app.git\n").as_deref(), Some("acme"));
        assert_eq!(remote_owner("git@github.com:acme/app.git").as_deref(), Some("acme"));
        assert_eq!(remote_owner("ssh://git@github.com/acme/app").as_deref(), Some("acme"));
        assert_eq!(remote_owner("/home/dev/app"), None);
    }
}

/// `GET /api/projects/managers` — whether each project's manager is in the middle of a turn, and when
/// its conversation last moved. A manager that has never been opened is simply not working.
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
        out.insert(project.name.clone(), json!({ "working": working, "updatedAt": updated }));
    }
    Json(Value::Object(out))
}
