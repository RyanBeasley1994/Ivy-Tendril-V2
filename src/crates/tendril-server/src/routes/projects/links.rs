//! What a project has hosted (`tendril_core::project_links`): the project page's Hosted card reads
//! these, and can add, correct or remove a link by hand. Agents write the same file with
//! `tendril link set`.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::Arc;
use tendril_core::config::load_config;
use tendril_core::project_links::{self, LinkInput};

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// The project's configured name, or a 404 when no such project exists.
fn project_name(state: &AppState, project: &str) -> Result<String, Response> {
    let settings = load_config(&state.config_path).map_err(|e| {
        error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load config: {e}"))
    })?;
    settings
        .projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(project))
        .map(|p| p.name.clone())
        .ok_or_else(|| error(StatusCode::NOT_FOUND, format!("Project '{project}' not found")))
}

pub async fn list_project_links(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    match project_name(&state, &name) {
        Ok(project) => Json(project_links::list(&state.tendril_home, &project)).into_response(),
        Err(response) => response,
    }
}

/// Upserts a link: by `slug` when the body names one, otherwise by URL.
pub async fn set_project_link(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(input): Json<LinkInput>,
) -> Response {
    let project = match project_name(&state, &name) {
        Ok(p) => p,
        Err(response) => return response,
    };
    match project_links::set(&state.tendril_home, &project, input) {
        Ok(link) => Json(link).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

pub async fn delete_project_link(
    State(state): State<Arc<AppState>>,
    Path((name, slug)): Path<(String, String)>,
) -> Response {
    let project = match project_name(&state, &name) {
        Ok(p) => p,
        Err(response) => return response,
    };
    match project_links::remove(&state.tendril_home, &project, &slug) {
        Ok(true) => Json(json!({ "deleted": slug })).into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, format!("No link '{slug}' in project {project}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
