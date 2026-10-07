//! `/api/push/*`: the installed web app's browser-push subscription. The page reads the daemon's VAPID
//! public key, subscribes against it with the browser's push service, and hands the subscription back.

use crate::state::AppState;
use crate::webpush::{self, Notice, Subscription};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

fn bad(message: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

/// The public key to subscribe against, and how many browsers are subscribed already.
pub async fn status_handler(State(state): State<Arc<AppState>>) -> Response {
    let home = state.tendril_home.clone();
    match tokio::task::spawn_blocking(move || webpush::public_key(&home).map(|k| (k, webpush::subscription_count(&home)))).await {
        Ok(Ok((key, subscribers))) => Json(json!({ "publicKey": key, "subscribers": subscribers })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct Keys {
    p256dh: String,
    auth: String,
}

/// `PushSubscription.toJSON()`, plus an optional label for the device.
#[derive(Deserialize)]
pub struct SubscribeBody {
    endpoint: String,
    keys: Keys,
    #[serde(default)]
    label: String,
}

pub async fn subscribe_handler(State(state): State<Arc<AppState>>, Json(body): Json<SubscribeBody>) -> Response {
    let home = state.tendril_home.clone();
    let sub = Subscription {
        endpoint: body.endpoint,
        p256dh: body.keys.p256dh,
        auth: body.keys.auth,
        label: body.label.chars().take(60).collect(),
        created: chrono::Utc::now().to_rfc3339(),
    };
    match tokio::task::spawn_blocking(move || webpush::subscribe(&home, sub).map(|_| webpush::subscription_count(&home))).await {
        Ok(Ok(subscribers)) => Json(json!({ "ok": true, "subscribers": subscribers })).into_response(),
        Ok(Err(e)) => bad(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct UnsubscribeBody {
    endpoint: String,
}

pub async fn unsubscribe_handler(State(state): State<Arc<AppState>>, Json(body): Json<UnsubscribeBody>) -> Response {
    let home = state.tendril_home.clone();
    match tokio::task::spawn_blocking(move || webpush::unsubscribe(&home, &body.endpoint).map(|_| webpush::subscription_count(&home))).await {
        Ok(Ok(subscribers)) => Json(json!({ "ok": true, "subscribers": subscribers })).into_response(),
        Ok(Err(e)) => bad(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

/// Sends a test notification to every subscribed browser, so setup can be checked end to end.
pub async fn test_handler(State(state): State<Arc<AppState>>) -> Response {
    let delivered = webpush::send_all(
        &state.tendril_home,
        &Notice { title: "Forge · test", body: "Notifications are working on this device.", urgent: false, tag: "forge-test", url: "/" },
    )
    .await;
    Json(json!({ "delivered": delivered })).into_response()
}
