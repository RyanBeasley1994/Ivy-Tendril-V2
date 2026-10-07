//! `/api/api-keys`: managing the public API's keys from the app. Behind the ordinary master-secret
//! layer (the web UI's login, or the desktop app's bearer), so only the owner can mint or revoke one;
//! a public-API key reaches none of these routes.

use crate::public_keys::{self, Scope};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// A key as the list shows it: never the hash, never the secret.
fn public_view(k: &public_keys::KeyRecord) -> serde_json::Value {
    json!({ "id": k.id, "name": k.name, "scope": k.scope, "prefix": k.prefix, "created": k.created })
}

pub async fn list_handler(State(state): State<Arc<AppState>>) -> Response {
    let home = state.tendril_home.clone();
    match tokio::task::spawn_blocking(move || public_keys::list(&home)).await {
        Ok(keys) => Json(json!({ "keys": keys.iter().map(public_view).collect::<Vec<_>>() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub name: String,
    /// May the key message managers as well as read?
    #[serde(default)]
    pub write: bool,
}

/// Creates a key. The response carries the secret `key`; this is the only time it is ever available.
pub async fn create_handler(State(state): State<Arc<AppState>>, Json(body): Json<CreateBody>) -> Response {
    let home = state.tendril_home.clone();
    let scope = if body.write { Scope::Write } else { Scope::Read };
    match tokio::task::spawn_blocking(move || public_keys::create(&home, &body.name, scope)).await {
        Ok(Ok((record, key))) => (
            StatusCode::CREATED,
            Json(json!({ "key": key, "record": public_view(&record) })),
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn revoke_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let home = state.tendril_home.clone();
    match tokio::task::spawn_blocking(move || public_keys::revoke(&home, &id)).await {
        Ok(Ok(true)) => Json(json!({ "ok": true })).into_response(),
        Ok(Ok(false)) => (StatusCode::NOT_FOUND, Json(json!({ "error": "no such key" }))).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}
