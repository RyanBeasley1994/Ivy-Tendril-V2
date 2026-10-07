//! `/api/public/v1`: the key layer, scopes, and what each route returns.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tendril_core::agents::providers::AgentProcessSpec;
use tendril_server::public_keys::{self, Scope};
use tendril_server::{create_router, AppState};
use tower::ServiceExt;

struct Harness {
    home: PathBuf,
    secret: String,
    router: Router,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn harness() -> Harness {
    let home = std::env::temp_dir().join(format!("forge-public-api-{}", uuid::Uuid::new_v4().simple()));
    let plans_dir = home.join("Plans");
    std::fs::create_dir_all(&plans_dir).unwrap();
    std::fs::write(
        home.join("config.yaml"),
        "codingAgent: claude\nprojects:\n  - name: Demo\n    color: Blue\n    repos: []\nverifications: []\n",
    )
    .unwrap();
    let secret = tendril_core::config::generate_bearer_secret();
    let mut state = AppState::with_plans_dir(home.clone(), plans_dir, secret.clone());
    // A manager "agent" that answers instantly, so a message produces a reply without a real model.
    state.chat_manager = Arc::new(
        tendril_core::chat::execution::ChatExecutionManager::new(home.clone()).with_spec_builder(Arc::new(
            |_agent, config| AgentProcessSpec {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), "echo '{\"delta\": \"On it.\"}'".to_string()],
                environment: HashMap::new(),
                working_directory: config.working_directory.clone(),
                stdin_content: None,
                redirect_stdin: false,
                temp_files: vec![],
            },
        )),
    );
    Harness { home, secret, router: create_router(Arc::new(state)) }
}

impl Harness {
    async fn call(&self, method: &str, uri: &str, key: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri).header("host", "localhost:5010");
        if let Some(key) = key {
            builder = builder.header("authorization", format!("Bearer {key}"));
        }
        let request = match body {
            Some(b) => builder.header("content-type", "application/json").body(Body::from(b.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    fn key(&self, name: &str, scope: Scope) -> String {
        public_keys::create(&self.home, name, scope).unwrap().1
    }
}

const PROJECTS: &str = "/api/public/v1/projects";

#[tokio::test]
async fn every_route_needs_a_valid_key_and_the_master_secret_is_not_one() {
    let h = harness();
    for uri in [PROJECTS, "/api/public/v1/projects/Demo", "/api/public/v1/projects/Demo/messages"] {
        assert_eq!(h.call("GET", uri, None, None).await.0, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(h.call("GET", uri, Some("fk_wrong"), None).await.0, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(h.call("GET", uri, Some("not-even-a-key"), None).await.0, StatusCode::UNAUTHORIZED, "{uri}");
    }
    // The daemon's own master secret opens the rest of the API, but not this one.
    assert_eq!(h.call("GET", PROJECTS, Some(&h.secret.clone()), None).await.0, StatusCode::UNAUTHORIZED);
    let key = h.key("reader", Scope::Read);
    assert_eq!(h.call("GET", PROJECTS, Some(&key), None).await.0, StatusCode::OK);
    // The key also works as X-Api-Key.
    let request = Request::builder()
        .uri(PROJECTS)
        .header("host", "localhost:5010")
        .header("x-api-key", &key)
        .body(Body::empty())
        .unwrap();
    assert_eq!(h.router.clone().oneshot(request).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn a_revoked_key_stops_working_at_once() {
    let h = harness();
    let key = h.key("temp", Scope::Read);
    assert_eq!(h.call("GET", PROJECTS, Some(&key), None).await.0, StatusCode::OK);
    assert!(public_keys::revoke(&h.home, "temp").unwrap());
    assert_eq!(h.call("GET", PROJECTS, Some(&key), None).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn projects_and_their_progress_are_listed() {
    let h = harness();
    let key = h.key("reader", Scope::Read);

    let (status, list) = h.call("GET", PROJECTS, Some(&key), None).await;
    assert_eq!(status, StatusCode::OK);
    let demo = &list["projects"][0];
    assert_eq!(demo["name"], "Demo");
    assert_eq!(demo["managerBusy"], false);
    assert_eq!(demo["activeMissions"], 0);
    assert_eq!(demo["needsYou"], 0);

    // The name matches however it is cased, and the detail carries the progress sections.
    let (status, detail) = h.call("GET", "/api/public/v1/projects/demo", Some(&key), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["name"], "Demo");
    for field in ["missions", "waitingOnYou", "jobs"] {
        assert!(detail[field].is_array(), "{field} is a list");
    }

    assert_eq!(h.call("GET", "/api/public/v1/projects/Nope", Some(&key), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_read_key_cannot_message_a_manager_and_input_is_validated() {
    let h = harness();
    let read = h.key("reader", Scope::Read);
    let write = h.key("writer", Scope::Write);
    let uri = "/api/public/v1/projects/Demo/messages";

    assert_eq!(h.call("POST", uri, Some(&read), Some(json!({ "text": "hi" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(h.call("POST", uri, Some(&write), Some(json!({ "text": "   " }))).await.0, StatusCode::BAD_REQUEST);
    let long = "x".repeat(8001);
    assert_eq!(h.call("POST", uri, Some(&write), Some(json!({ "text": long }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        h.call("POST", "/api/public/v1/projects/Nope/messages", Some(&write), Some(json!({ "text": "hi" })))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_message_reaches_the_manager_and_its_reply_is_read_back_by_cursor() {
    let h = harness();
    let write = h.key("writer", Scope::Write);
    let uri = "/api/public/v1/projects/Demo/messages";

    // Nobody has spoken to the manager yet.
    let (status, empty) = h.call("GET", uri, Some(&write), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["messages"], json!([]));

    let (status, sent) = h.call("POST", uri, Some(&write), Some(json!({ "text": "How is it going?" }))).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{sent}");
    assert_eq!(sent["accepted"], true);
    assert_eq!(sent["queued"], false);
    assert!(sent["cursor"].is_null(), "there was nothing before this message");

    // Poll until the manager has answered. The briefing never appears; the question and the reply do.
    let mut messages = Vec::new();
    for _ in 0..50 {
        let (_, page) = h.call("GET", uri, Some(&write), None).await;
        messages = page["messages"].as_array().cloned().unwrap_or_default();
        if messages.iter().any(|m| m["role"] == "assistant") && page["managerBusy"] == false {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let roles: Vec<_> = messages.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["user", "assistant"], "{messages:?}");
    assert_eq!(messages[0]["text"], "How is it going?");
    assert!(messages[1]["text"].as_str().unwrap().contains("On it."));

    // `after=<id>` returns only what came later.
    let first = messages[0]["id"].as_str().unwrap();
    let (_, later) = h.call("GET", &format!("{uri}?after={first}"), Some(&write), None).await;
    assert_eq!(later["messages"].as_array().unwrap().len(), 1);
    assert_eq!(later["messages"][0]["role"], "assistant");
    let last = messages[1]["id"].as_str().unwrap();
    let (_, none) = h.call("GET", &format!("{uri}?after={last}"), Some(&write), None).await;
    assert_eq!(none["messages"], json!([]));
    assert_eq!(h.call("GET", &format!("{uri}?after=nope"), Some(&write), None).await.0, StatusCode::NOT_FOUND);

    // The project's pulse now shows the manager's last reply.
    let (_, detail) = h.call("GET", "/api/public/v1/projects/Demo", Some(&write), None).await;
    assert!(detail["lastManagerReply"]["text"].as_str().unwrap().contains("On it."));
}
