//! The public API: `/api/public/v1`.
//!
//! A small, stable surface for scripts and other apps: list the projects, see how one is getting on,
//! read its manager's conversation, and (with a `write` key) message the manager. It is reachable on
//! the same URL as the web UI, so the front proxy lets this prefix through without the browser login
//! and the daemon does the authenticating here: every request must carry an API key made with
//! `tendril api-key create` (see [`crate::public_keys`]), as `Authorization: Bearer fk_...` or
//! `X-Api-Key: fk_...`. The daemon's master secret is not accepted: this surface has its own,
//! narrower credential on purpose.
//!
//! Nothing here can read files, run commands directly, or change configuration. The one action,
//! messaging a manager, hands the text to the project's Factory Manager exactly as typing it in the
//! app would, so whatever the manager is allowed to do, it decides.

use crate::public_keys::{self, KeyRecord};
use crate::state::AppState;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tendril_core::chat::execution::ChatTurnOptions;
use tendril_core::chat::manager_brief::manager_session_id;
use tendril_core::chat::models::ChatQueuedItem;
use tendril_core::config::load_config;
use tendril_core::missions::model::{MilestoneState, MissionState};
use tendril_core::missions::store::list_missions;
use uuid::Uuid;

/// Requests per key per minute. Generous for polling, small enough that a runaway loop is stopped.
const REQUESTS_PER_MINUTE: u32 = 240;
/// The longest message a caller may send a manager.
const MAX_MESSAGE_CHARS: usize = 8000;

static WINDOWS: Mutex<Option<HashMap<String, (Instant, u32)>>> = Mutex::new(None);

fn within_rate_limit(key_id: &str) -> bool {
    let mut guard = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    let entry = map.entry(key_id.to_string()).or_insert((now, 0));
    if now.duration_since(entry.0) >= Duration::from_secs(60) {
        *entry = (now, 0);
    }
    entry.1 += 1;
    entry.1 <= REQUESTS_PER_MINUTE
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/public/v1/projects", get(list_projects))
        .route("/api/public/v1/projects/:name", get(project_progress))
        .route(
            "/api/public/v1/projects/:name/messages",
            get(read_messages).post(send_message),
        )
        .layer(axum::middleware::from_fn_with_state(state, require_key))
}

/// Checks the key, applies the rate limit, and hands the key's record to the handler.
async fn require_key(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let presented = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| req.headers().get("x-api-key").and_then(|v| v.to_str().ok()))
        .unwrap_or_default()
        .trim()
        .to_string();
    let home = state.tendril_home.clone();
    let record = tokio::task::spawn_blocking(move || public_keys::verify(&home, &presented))
        .await
        .ok()
        .flatten();
    let Some(record) = record else {
        return error(
            StatusCode::UNAUTHORIZED,
            "Missing or invalid API key. Send Authorization: Bearer <key> (create one with `tendril api-key create`).",
        );
    };
    if !within_rate_limit(&record.id) {
        return error(StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded; try again in a minute.");
    }
    req.extensions_mut().insert(record);
    next.run(req).await
}

fn project_names(state: &AppState) -> Result<Vec<(String, Vec<String>)>, Response> {
    let settings = load_config(&state.config_path)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load config: {e}")))?;
    let home = state.tendril_home.to_string_lossy().to_string();
    Ok(settings
        .projects
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.repos
                    .iter()
                    .map(|r| tendril_core::config::expand_variables(&r.path, &home))
                    .collect(),
            )
        })
        .collect())
}

fn find_project(state: &AppState, name: &str) -> Result<(String, Vec<String>), Response> {
    project_names(state)?
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .ok_or_else(|| error(StatusCode::NOT_FOUND, format!("Project '{name}' not found")))
}

/// What a person reads of the conversation: their own and the manager's words, not the briefing or
/// injected system events.
fn visible_messages(session: &tendril_core::chat::ChatSession) -> Vec<&tendril_core::chat::ChatMessage> {
    session
        .messages
        .iter()
        .filter(|m| (m.role == "user" || m.role == "assistant") && !m.content.trim().is_empty())
        .collect()
}

fn message_json(m: &tendril_core::chat::ChatMessage) -> Value {
    json!({ "id": m.id, "role": m.role, "text": m.content, "at": m.timestamp })
}

async fn last_reply(state: &AppState, project: &str) -> Option<Value> {
    let session = state.chat_manager.get_session(&manager_session_id(project)).await.ok()?;
    let reply = visible_messages(&session).into_iter().rev().find(|m| m.role == "assistant")?;
    Some(json!({
        "id": reply.id,
        "at": reply.timestamp,
        "text": crate::push::clip(&crate::push::plain(&reply.content), 400),
    }))
}

struct Pulse {
    missions: Vec<tendril_core::missions::model::MissionFile>,
    jobs: Vec<tendril_core::models::JobItem>,
}

async fn pulse(state: &AppState, project: &str) -> Pulse {
    let missions = list_missions(&state.mission_driver.paths().missions_dir)
        .into_iter()
        .filter(|f| f.mission.project.eq_ignore_ascii_case(project))
        .collect();
    let jobs = state
        .job_manager
        .list_non_terminal_jobs()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|j| j.project.eq_ignore_ascii_case(project))
        .collect();
    Pulse { missions, jobs }
}

fn needs_you(f: &tendril_core::missions::model::MissionFile) -> bool {
    matches!(f.mission.state, MissionState::AwaitingApproval | MissionState::Paused)
}

fn mission_json(f: &tendril_core::missions::model::MissionFile) -> Value {
    let m = &f.mission;
    let live: Vec<_> = m.milestones.iter().filter(|x| x.state != MilestoneState::Skipped).collect();
    let passed = live.iter().filter(|x| x.state == MilestoneState::Passed).count();
    let current = live
        .iter()
        .find(|x| matches!(x.state, MilestoneState::Executing | MilestoneState::Judging))
        .map(|x| json!({ "id": x.id, "title": x.title, "state": x.state.as_str() }));
    json!({
        "id": f.id,
        "title": m.title,
        "state": m.state.as_str(),
        "needsYou": needs_you(f),
        "pauseReason": m.pause_reason,
        "branch": m.branch,
        "milestones": { "total": live.len(), "passed": passed },
        "currentMilestone": current,
        "cost": m.cost,
        "updated": m.updated,
    })
}

fn job_json(j: &tendril_core::models::JobItem) -> Value {
    json!({
        "id": j.id,
        "type": j.job_type,
        "status": j.status,
        "plan": j.plan_file,
        "startedAt": j.started_at,
    })
}

async fn summary(state: &AppState, name: &str, repos: &[String]) -> Value {
    let p = pulse(state, name).await;
    let active = p.missions.iter().filter(|f| !f.mission.state.is_terminal()).count();
    let attention = crate::attention::for_project(state, name).await;
    json!({
        "name": name,
        "repos": repos,
        "managerBusy": state.chat_manager.is_generating(&manager_session_id(name)).await,
        "activeMissions": active,
        // Missions waiting on you, plus a question the manager asked that you have not answered.
        "needsYou": attention.count(),
        "managerAsked": attention.asked,
        "runningJobs": p.jobs.len(),
        "lastManagerReply": last_reply(state, name).await,
    })
}

/// `GET /projects`: every project with a one-line pulse.
async fn list_projects(State(state): State<Arc<AppState>>) -> Response {
    let names = match project_names(&state) {
        Ok(n) => n,
        Err(r) => return r,
    };
    let mut projects = Vec::with_capacity(names.len());
    for (name, repos) in &names {
        projects.push(summary(&state, name, repos).await);
    }
    Json(json!({ "projects": projects })).into_response()
}

/// `GET /projects/:name`: the pulse plus every live mission, the running jobs and what waits on a person.
async fn project_progress(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let (name, repos) = match find_project(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let mut value = summary(&state, &name, &repos).await;
    let p = pulse(&state, &name).await;
    let mut missions: Vec<_> = p.missions.iter().collect();
    // Live ones first, then the most recently touched finished ones, capped.
    missions.sort_by(|a, b| {
        a.mission
            .state
            .is_terminal()
            .cmp(&b.mission.state.is_terminal())
            .then(b.mission.updated.cmp(&a.mission.updated))
    });
    let live = missions.iter().filter(|f| !f.mission.state.is_terminal()).count();
    missions.truncate(live.max(0) + 10);
    value["missions"] = json!(missions.iter().map(|f| mission_json(f)).collect::<Vec<_>>());
    value["waitingOnYou"] = json!(p
        .missions
        .iter()
        .filter(|f| needs_you(f))
        .map(|f| json!({
            "mission": f.id,
            "title": f.mission.title,
            "state": f.mission.state.as_str(),
            "reason": if f.mission.state == MissionState::Paused {
                f.mission.pause_reason.clone()
            } else {
                Some("waiting for your approval".to_string())
            },
        }))
        .collect::<Vec<_>>());
    value["jobs"] = json!(p.jobs.iter().map(job_json).collect::<Vec<_>>());
    Json(value).into_response()
}

#[derive(Debug, Deserialize, Default)]
pub struct MessagesQuery {
    /// Only messages after this id (oldest first): the polling cursor.
    pub after: Option<String>,
    /// Only messages before this id: paging back.
    pub before: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /projects/:name/messages`: the manager's conversation. With no query, the latest `limit`
/// messages; `after=<id>` is what a poller sends to get only what is new.
async fn read_messages(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> Response {
    let (name, _) = match find_project(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let busy = state.chat_manager.is_generating(&manager_session_id(&name)).await;
    let Ok(session) = state.chat_manager.get_session(&manager_session_id(&name)).await else {
        // No conversation yet: the manager has simply never been spoken to.
        return Json(json!({ "messages": [], "managerBusy": busy, "hasMore": false })).into_response();
    };
    let all = visible_messages(&session);
    let limit = query.limit.unwrap_or(30).clamp(1, 100);
    let position = |id: &str| all.iter().position(|m| m.id == id);

    let (slice, has_more): (&[&tendril_core::chat::ChatMessage], bool) = match (&query.after, &query.before) {
        (Some(id), _) => match position(id) {
            Some(at) => {
                let rest = &all[at + 1..];
                (&rest[..rest.len().min(limit)], rest.len() > limit)
            }
            None => return error(StatusCode::NOT_FOUND, "that message is not in this conversation"),
        },
        (None, Some(id)) => match position(id) {
            Some(at) => {
                let start = at.saturating_sub(limit);
                (&all[start..at], start > 0)
            }
            None => return error(StatusCode::NOT_FOUND, "that message is not in this conversation"),
        },
        (None, None) => {
            let start = all.len().saturating_sub(limit);
            (&all[start..], start > 0)
        }
    };
    Json(json!({
        "messages": slice.iter().map(|m| message_json(m)).collect::<Vec<_>>(),
        "managerBusy": busy,
        "hasMore": has_more,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct SendBody {
    pub text: String,
}

/// `POST /projects/:name/messages`: tells the manager something. The manager answers in its own time:
/// the response carries a `cursor` (the id of the last message before this one) to poll
/// `GET .../messages?after=<cursor>` with. A manager already mid-turn queues the message and takes it
/// up when it finishes, as it does in the app.
async fn send_message(
    State(state): State<Arc<AppState>>,
    Extension(key): Extension<KeyRecord>,
    Path(name): Path<String>,
    Json(body): Json<SendBody>,
) -> Response {
    if !key.scope.allows_write() {
        return error(StatusCode::FORBIDDEN, "This key is read-only. Create one with --write to message managers.");
    }
    let text = body.text.trim();
    if text.is_empty() {
        return error(StatusCode::BAD_REQUEST, "text must not be empty");
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return error(StatusCode::BAD_REQUEST, format!("text is longer than {MAX_MESSAGE_CHARS} characters"));
    }
    let (name, _) = match find_project(&state, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };

    // Make sure the manager exists and has its current briefing, the way opening it in the app does.
    let opened = crate::routes::projects::get_or_create_project_manager(State(state.clone()), Path(name.clone()))
        .await
        .into_response();
    if !opened.status().is_success() {
        return opened;
    }

    let id = manager_session_id(&name);
    let cursor = state
        .chat_manager
        .get_session(&id)
        .await
        .ok()
        .and_then(|s| visible_messages(&s).last().map(|m| m.id.clone()));

    if state.chat_manager.is_generating(&id).await {
        let item = ChatQueuedItem {
            id: Uuid::new_v4().to_string(),
            prompt: text.to_string(),
            attachments: None,
            created_at: chrono::Utc::now(),
            role: None,
        };
        state.chat_manager.enqueue_message(&id, item).await;
        return (StatusCode::ACCEPTED, Json(json!({ "accepted": true, "queued": true, "cursor": cursor })))
            .into_response();
    }
    match state.chat_manager.start_session_turn(&id, text, ChatTurnOptions::default()).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "accepted": true, "queued": false, "cursor": cursor })))
            .into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}
