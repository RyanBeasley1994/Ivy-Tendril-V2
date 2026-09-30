//! `/api/missions` over HTTP: creating a mission makes its integration plan and starts the
//! orchestrator's planning job in the same request; the control routes answer with the mission; bad
//! input is a 4xx that says what was wrong.
//!
//! The fixture home has no `Promptwares` folder, so the planning job stops at the promptware gate
//! without spawning anything. Whether it has already failed by the time the response is built is a
//! race, so the assertions accept either side of it.

use std::path::PathBuf;
use std::sync::Arc;
use tendril_core::config::MasterGuard;
use tendril_server::{create_router, AppState};

struct TestServer {
    tendril_home: PathBuf,
    port: u16,
    secret: String,
    _guard: MasterGuard,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = std::fs::remove_dir_all(&self.tendril_home);
    }
}

async fn start_test_server() -> TestServer {
    let tendril_home = std::env::temp_dir().join(format!("tendril-mission-routes-{}", uuid::Uuid::new_v4().simple()));
    let repo = tendril_home.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        tendril_home.join("config.yaml"),
        format!("projects:\n  - name: P\n    repos:\n      - path: {}\n", repo.display()),
    )
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let secret = tendril_core::config::generate_bearer_secret();
    let guard = MasterGuard::acquire(&tendril_home, port, &secret, "127.0.0.1", "http").unwrap();
    let plans_dir = tendril_home.join("Plans");
    std::fs::create_dir_all(&plans_dir).unwrap();
    let state = Arc::new(AppState::with_plans_dir(tendril_home.clone(), plans_dir, secret.clone()));
    let app = create_router(state);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    TestServer { tendril_home, port, secret, _guard: guard, shutdown_tx: Some(shutdown_tx) }
}

impl TestServer {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
    async fn post(&self, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        let resp = reqwest::Client::new().post(self.url(path)).bearer_auth(&self.secret).json(&body).send().await.unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or_default())
    }
    async fn get(&self, path: &str) -> (u16, serde_json::Value) {
        let resp = reqwest::Client::new().get(self.url(path)).bearer_auth(&self.secret).send().await.unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or_default())
    }
}

#[tokio::test]
async fn creating_a_mission_makes_its_integration_plan_and_starts_planning() {
    let server = start_test_server().await;
    let (status, body) = server
        .post("/api/missions", serde_json::json!({ "title": "Add SSO", "goal": "Users sign in with SSO", "project": "P" }))
        .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["id"], "00001");
    let state = body["state"].as_str().unwrap();
    assert!(matches!(state, "Planning" | "Paused"), "state {state}");
    assert_eq!(body["jobs"].as_array().map(|a| a.len()), Some(1), "the planning job was started: {body}");
    assert_eq!(body["integrationPlanState"], "Blocked");
    let plan = body["integrationPlan"].as_str().unwrap();
    assert!(server.tendril_home.join("Plans").join(plan).join("plan.yaml").is_file());

    let (status, list) = server.get("/api/missions").await;
    assert_eq!(status, 200);
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (status, one) = server.get("/api/missions/1").await;
    assert_eq!(status, 200);
    assert_eq!(one["title"], "Add SSO");
}

#[tokio::test]
async fn bad_requests_say_what_was_wrong() {
    let server = start_test_server().await;
    let (status, body) = server
        .post("/api/missions", serde_json::json!({ "title": "X", "goal": "Y", "project": "Nope" }))
        .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("Nope"));

    let (status, _) = server.post("/api/missions", serde_json::json!({ "title": "", "goal": "Y", "project": "P" })).await;
    assert_eq!(status, 400);

    let (status, _) = server.get("/api/missions/42").await;
    assert_eq!(status, 404);

    let (_, created) = server.post("/api/missions", serde_json::json!({ "title": "T", "goal": "G", "project": "P" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, body) = server.post(&format!("/api/missions/{id}/approve"), serde_json::json!({})).await;
    assert_eq!(status, 409, "cannot approve a mission with no milestones: {body}");

    let (status, body) = server.post(&format!("/api/missions/{id}/cancel"), serde_json::json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(body["state"], "Cancelled");
    // Released by the dependent-release pass the cancelled job triggers, or by the next unblock
    // pass; either way the gate no longer holds it.
    let plan_state = body["integrationPlanState"].as_str().unwrap();
    assert!(matches!(plan_state, "Draft" | "Blocked"), "{plan_state}");
}
