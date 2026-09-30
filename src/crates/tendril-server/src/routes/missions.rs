//! `/api/missions`: create, list and steer missions. Every mutating route follows its write with a
//! driver reconcile, so the next step starts in the same request rather than on the next tick.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tendril_core::error::TendrilError;
use tendril_core::missions::service::{self, MilestoneInput, MilestoneScope, NewMission};
use tendril_core::missions::store::{list_missions, read_mission_file, resolve_mission_folder};

fn error_response(e: TendrilError) -> Response {
    let status = match &e {
        TendrilError::MissionNotFound(_) => StatusCode::NOT_FOUND,
        TendrilError::Validation(_) | TendrilError::ProjectNotFound(_) => StatusCode::BAD_REQUEST,
        TendrilError::Mission(_) | TendrilError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": e.to_string() }))).into_response()
}

fn folder(state: &AppState, id: &str) -> Result<PathBuf, Response> {
    resolve_mission_folder(id, &state.mission_driver.paths().missions_dir).map_err(error_response)
}

/// The mission as JSON, with the integration plan's state alongside so the UI can link to it.
fn mission_json(state: &AppState, folder: &std::path::Path) -> Response {
    match read_mission_file(folder) {
        Ok(file) => {
            let integration_state = file
                .mission
                .integration_plan
                .as_ref()
                .map(|p| state.plans_dir.join(p))
                .and_then(|p| tendril_core::plans::reader::read_plan_yaml(&p).ok())
                .map(|(plan, _)| plan.state);
            let mut value = serde_json::to_value(&file).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("integrationPlanState".into(), json!(integration_state));
            }
            (StatusCode::OK, Json(value)).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn reconcile(state: &AppState, folder: &std::path::Path) {
    if let Err(e) = state.mission_driver.reconcile(folder).await {
        tracing::warn!("Mission {}: {}", folder.display(), e);
    }
}

/// `GET /api/missions`
pub async fn list_missions_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(list_missions(&state.mission_driver.paths().missions_dir))
}

/// `POST /api/missions`
pub async fn create_mission_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NewMission>,
) -> Response {
    let settings = state.settings_snapshot().settings.clone();
    let created = match service::create(state.mission_driver.paths(), &settings, req) {
        Ok(c) => c,
        Err(e) => return error_response(e),
    };
    let folder = PathBuf::from(&created.folder_path);
    reconcile(&state, &folder).await;
    let mut response = mission_json(&state, &folder);
    *response.status_mut() = StatusCode::CREATED;
    response
}

/// `GET /api/missions/:id`
pub async fn get_mission_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match folder(&state, &id) {
        Ok(f) => mission_json(&state, &f),
        Err(r) => r,
    }
}

/// `POST /api/missions/:id/approve`
pub async fn approve_mission_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(e) = service::approve(&f) {
        return error_response(e);
    }
    reconcile(&state, &f).await;
    mission_json(&state, &f)
}

#[derive(Debug, Default, Deserialize)]
pub struct PauseRequest {
    pub reason: Option<String>,
}

/// `POST /api/missions/:id/pause` — also stops the job the mission is running.
pub async fn pause_mission_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<PauseRequest>>,
) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let reason = body.and_then(|Json(b)| b.reason);
    if let Err(e) = state.mission_driver.pause(&f, reason.as_deref()).await {
        return error_response(e);
    }
    mission_json(&state, &f)
}

/// `POST /api/missions/:id/resume`
pub async fn resume_mission_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(e) = service::resume(&f) {
        return error_response(e);
    }
    reconcile(&state, &f).await;
    mission_json(&state, &f)
}

/// `POST /api/missions/:id/cancel` — also stops the job the mission is running.
pub async fn cancel_mission_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(e) = state.mission_driver.cancel(&f).await {
        return error_response(e);
    }
    mission_json(&state, &f)
}

/// `POST /api/missions/:id/complete` — the operator marks it done; stops any job still running.
pub async fn complete_mission_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(e) = state.mission_driver.complete(&f).await {
        return error_response(e);
    }
    mission_json(&state, &f)
}

/// `POST /api/missions/:id/reconcile` — take the next step now. What the CLI calls after a file edit.
pub async fn reconcile_mission_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    reconcile(&state, &f).await;
    mission_json(&state, &f)
}

#[derive(Debug, Default, Deserialize)]
pub struct BudgetRequest {
    #[serde(rename = "maxAttempts")]
    pub max_attempts: Option<u32>,
    #[serde(rename = "maxReplans")]
    pub max_replans: Option<u32>,
    #[serde(rename = "maxCost")]
    pub max_cost: Option<f64>,
}

/// `PUT /api/missions/:id/budget`
pub async fn budget_mission_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<BudgetRequest>,
) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(e) = service::set_budget(&f, req.max_attempts, req.max_replans, req.max_cost) {
        return error_response(e);
    }
    reconcile(&state, &f).await;
    mission_json(&state, &f)
}

/// `PUT /api/missions/:id/agents` — the harness per role; applies from the next job the mission starts.
pub async fn set_agents_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(agents): Json<tendril_core::missions::model::MissionAgents>,
) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(e) = service::set_agents(&f, agents) {
        return error_response(e);
    }
    mission_json(&state, &f)
}

#[derive(Debug, Deserialize)]
pub struct MilestonesRequest {
    pub milestones: Vec<MilestoneInput>,
}

/// `PUT /api/missions/:id/milestones` — the operator editing the milestone plan before approving it.
pub async fn set_milestones_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MilestonesRequest>,
) -> Response {
    let f = match folder(&state, &id) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let raw = match serde_json::to_string(&req.milestones) {
        Ok(r) => r,
        Err(e) => return error_response(TendrilError::Json(e)),
    };
    let inputs = match service::parse_milestones(&raw) {
        Ok(i) => i,
        Err(e) => return error_response(e),
    };
    if let Err(e) = service::set_milestones(&f, &state.plans_dir, inputs, MilestoneScope::All) {
        return error_response(e);
    }
    mission_json(&state, &f)
}
