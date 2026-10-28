//! `/api/support`: the Support page's view of Propriotec Operations' tickets
//! (`tendril_core::support`). The daemon holds the Operations login; the webview only ever sees
//! tickets and [`tendril_core::support::SupportStatus`].

use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tendril_core::config::load_config;
use tendril_core::error::TendrilError;
use tendril_core::support;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn failed(e: TendrilError) -> Response {
    let status = match e {
        TendrilError::Validation(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    };
    error(status, e.to_string())
}

fn answer(result: Result<serde_json::Value, TendrilError>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => failed(e),
    }
}

pub async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    Json(support::status(&state.tendril_home)).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    #[serde(default)]
    pub base_url: String,
    pub email: String,
    pub password: String,
}

pub async fn connect(State(state): State<Arc<AppState>>, Json(req): Json<ConnectRequest>) -> Response {
    match support::connect(&state.tendril_home, &req.base_url, &req.email, &req.password).await {
        Ok(status) => Json(status).into_response(),
        Err(e) => failed(e),
    }
}

pub async fn disconnect(State(state): State<Arc<AppState>>) -> Response {
    match support::disconnect(&state.tendril_home) {
        Ok(()) => Json(support::status(&state.tendril_home)).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    #[serde(default)]
    pub include_done: bool,
}

pub async fn list_tickets(State(state): State<Arc<AppState>>, Query(q): Query<ListQuery>) -> Response {
    answer(support::list_tickets(&state.tendril_home, q.include_done).await)
}

/// The sidebar badge. An unconnected daemon answers zero rather than an error, so the badge is
/// simply absent until Support is set up.
pub async fn count(State(state): State<Arc<AppState>>) -> Response {
    if !support::status(&state.tendril_home).configured {
        return Json(json!({ "count": 0, "configured": false })).into_response();
    }
    match support::unread_count(&state.tendril_home).await {
        Ok(count) => Json(json!({ "count": count, "configured": true })).into_response(),
        Err(e) => failed(e),
    }
}

pub async fn get_ticket(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    answer(support::get_ticket(&state.tendril_home, &id).await)
}

pub async fn mark_read(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    answer(support::mark_read(&state.tendril_home, &id).await.map(|()| json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyRequest {
    pub content: String,
    #[serde(default)]
    pub internal: bool,
}

pub async fn reply(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<ReplyRequest>) -> Response {
    answer(support::reply(&state.tendril_home, &id, &req.content, req.internal).await)
}

#[derive(Debug, Deserialize)]
pub struct StatusRequest {
    pub status: String,
}

pub async fn set_status(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<StatusRequest>) -> Response {
    answer(support::set_status(&state.tendril_home, &id, &req.status).await)
}

#[derive(Debug, Default, Deserialize)]
pub struct CloseRequest {
    #[serde(default)]
    pub message: Option<String>,
}

pub async fn close(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<CloseRequest>) -> Response {
    answer(support::close(&state.tendril_home, &id, req.message.as_deref()).await.map(|()| json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffRequest {
    pub project: String,
    #[serde(default)]
    pub instructions: String,
}

/// Hands a ticket to a project's manager: the manager gets the whole ticket and how to work it, the
/// ticket gets an internal note and moves to In Progress. The partner sees neither.
pub async fn handoff(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<HandoffRequest>) -> Response {
    let home = &state.tendril_home;
    let Some(project) = load_config(&state.config_path)
        .ok()
        .and_then(|s| s.projects.into_iter().find(|p| p.name.eq_ignore_ascii_case(req.project.trim())))
        .map(|p| p.name)
    else {
        return error(StatusCode::NOT_FOUND, format!("Project '{}' not found", req.project));
    };
    let ticket = match support::get_ticket(home, &id).await {
        Ok(t) => t,
        Err(e) => return failed(e),
    };
    let prompt = support::handoff_prompt(&ticket, &project, &req.instructions);
    if let Err(e) = crate::telegram::tell_manager(&state, &project, &prompt).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    let handoff = match support::record_handoff(home, &id, &project) {
        Ok(h) => h,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    // The manager already has it; a failure to note it on the ticket should not undo that.
    let note = format!("Handed to the Forge manager for project {project}. Internal notes from it follow.");
    if let Err(e) = support::reply(home, &id, &note, true).await {
        tracing::warn!("support handoff note on {id} failed: {e}");
    }
    if ticket["status"].as_str().is_some_and(|s| s == "OPEN" || s == "PENDING") {
        if let Err(e) = support::set_status(home, &id, "IN_PROGRESS").await {
            tracing::warn!("support handoff status on {id} failed: {e}");
        }
    }
    Json(handoff).into_response()
}
