//! The Telegram bot end to end, against a stand-in for Telegram's API: setting the bot, pairing one
//! account, and the rule the whole thing rests on, that nobody else gets an answer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tendril_core::agents::providers::AgentProcessSpec;
use tendril_server::{create_router, AppState};
use tower::ServiceExt;

#[derive(Default)]
struct FakeTelegram {
    /// Updates waiting to be handed to the daemon.
    inbox: Vec<Value>,
    /// Every message the daemon sent: `(chat_id, text)`.
    sent: Vec<(i64, String)>,
    next_update: i64,
}

type Fake = Arc<Mutex<FakeTelegram>>;

async fn fake_api(State(fake): State<Fake>, Path((_bot, method)): Path<(String, String)>, Json(body): Json<Value>) -> Json<Value> {
    match method.as_str() {
        "getMe" => Json(json!({ "ok": true, "result": { "username": "ForgeTestBot" } })),
        "getUpdates" => {
            let offset = body["offset"].as_i64().unwrap_or(0);
            let ready: Vec<Value> = {
                let mut f = fake.lock().unwrap();
                f.inbox.retain(|u| u["update_id"].as_i64().unwrap_or(0) >= offset);
                f.inbox.clone()
            };
            if ready.is_empty() {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            Json(json!({ "ok": true, "result": ready }))
        }
        "sendMessage" => {
            let mut f = fake.lock().unwrap();
            f.sent.push((body["chat_id"].as_i64().unwrap_or(0), body["text"].as_str().unwrap_or("").to_string()));
            let id = f.sent.len() as i64;
            Json(json!({ "ok": true, "result": { "message_id": id } }))
        }
        _ => Json(json!({ "ok": false, "description": "unknown method" })),
    }
}

fn says(fake: &Fake, user: i64, chat: i64, kind: &str, text: &str) {
    let mut f = fake.lock().unwrap();
    f.next_update += 1;
    let update_id = f.next_update;
    f.inbox.push(json!({
        "update_id": update_id,
        "message": { "chat": { "id": chat, "type": kind }, "from": { "id": user, "first_name": "Ryan", "username": "ryan" }, "text": text }
    }));
}

/// Waits until the daemon has taken every update and `done` holds, or fails after a few seconds.
async fn until(fake: &Fake, what: &str, done: impl Fn(&FakeTelegram) -> bool) {
    for _ in 0..150 {
        {
            let f = fake.lock().unwrap();
            if done(&f) {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!("never happened: {what}; sent so far: {:?}", fake.lock().unwrap().sent);
}

async fn call(router: &Router, method: &str, uri: &str, secret: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri).header("host", "localhost:5010");
    if let Some(secret) = secret {
        builder = builder.header("authorization", format!("Bearer {secret}"));
    }
    let request = match body {
        Some(b) => builder.header("content-type", "application/json").body(Body::from(b.to_string())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_bot_pairs_one_account_talks_to_a_manager_and_answers_nobody_else() {
    // A stand-in for api.telegram.org.
    let fake: Fake = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let api = Router::new().route("/:bot/:method", post(fake_api)).with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });
    std::env::set_var("TENDRIL_TELEGRAM_API", format!("http://{address}"));

    let home = std::env::temp_dir().join(format!("forge-telegram-{}", uuid::Uuid::new_v4().simple()));
    let plans_dir = home.join("Plans");
    std::fs::create_dir_all(&plans_dir).unwrap();
    std::fs::write(
        home.join("config.yaml"),
        "codingAgent: claude\nprojects:\n  - name: Demo-App\n    color: Blue\n    repos: []\nverifications: []\n",
    )
    .unwrap();
    let secret = tendril_core::config::generate_bearer_secret();
    let mut state = AppState::with_plans_dir(home.clone(), plans_dir, secret.clone());
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
    let state = Arc::new(state);
    let router = create_router(state.clone());
    tendril_server::telegram::spawn(state.clone());

    // Setting the bot: the token is checked, never shown back, and a code to pair with is issued.
    assert_ne!(call(&router, "GET", "/api/telegram", None, None).await.0, StatusCode::OK, "settings need the owner's secret");
    let (status, set) = call(&router, "PUT", "/api/telegram", Some(&secret), Some(json!({ "token": "123:abc" }))).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(set["botUsername"], "ForgeTestBot");
    assert_eq!(set["paired"], false);
    assert!(!set.to_string().contains("123:abc"), "the token must not come back: {set}");
    let code = set["pairCode"].as_str().unwrap().to_string();

    // A stranger who found the bot: a wrong code, a command, a message. Not one reply.
    says(&fake, 99, 99, "private", "/start 000000x");
    says(&fake, 99, 99, "private", "/status");
    says(&fake, 99, 99, "private", "delete everything");
    // The right code from a group does not pair either: the bot belongs in a private chat.
    says(&fake, 7, -500, "group", &format!("/start {code}"));
    until(&fake, "the daemon read the strangers' messages", |f| f.inbox.is_empty()).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(fake.lock().unwrap().sent.is_empty(), "nobody unpaired gets an answer: {:?}", fake.lock().unwrap().sent);

    // The operator pairs their own account.
    says(&fake, 7, 7, "private", &format!("/start {code}"));
    until(&fake, "pairing is confirmed", |f| f.sent.iter().any(|(chat, text)| *chat == 7 && text.starts_with("Paired."))).await;
    let (_, paired) = call(&router, "GET", "/api/telegram", Some(&secret), None).await;
    assert_eq!(paired["paired"], true);
    assert_eq!(paired["pairedWith"], "Ryan (@ryan)");
    assert!(paired["pairCode"].is_null(), "the code is spent: {paired}");

    // Once paired, the code is gone and a stranger still gets nothing, even with the old code.
    let before = fake.lock().unwrap().sent.len();
    says(&fake, 99, 99, "private", &format!("/start {code}"));
    says(&fake, 99, 99, "private", "/manager demo");
    until(&fake, "the daemon read them", |f| f.inbox.is_empty()).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(fake.lock().unwrap().sent.len(), before, "a stranger is still ignored");

    // Typing without picking a manager is not sent anywhere.
    says(&fake, 7, 7, "private", "hello?");
    until(&fake, "told to pick a manager", |f| f.sent.iter().any(|(_, t)| t.contains("/manager <project>"))).await;

    // A loose name finds the project, and from then on the conversation flows both ways.
    says(&fake, 7, 7, "private", "/manager demo app");
    until(&fake, "bound to the manager", |f| f.sent.iter().any(|(_, t)| t.starts_with("Talking to Demo-App's manager"))).await;
    says(&fake, 7, 7, "private", "ship the login page");
    until(&fake, "the manager's reply came back", |f| f.sent.iter().any(|(chat, t)| *chat == 7 && t == "On it.")).await;
    let sent = fake.lock().unwrap().sent.clone();
    assert!(sent.iter().all(|(chat, _)| *chat == 7), "everything goes to the one paired chat: {sent:?}");
    assert!(!sent.iter().any(|(_, t)| t.contains("ship the login page")), "their own words are not echoed back: {sent:?}");
    assert!(!sent.iter().any(|(_, t)| t.contains("You are the Factory Manager")), "the briefing is not forwarded");

    says(&fake, 7, 7, "private", "/end");
    until(&fake, "unbound", |f| f.sent.iter().any(|(_, t)| t.starts_with("Stopped talking to Demo-App"))).await;

    // Unpairing forgets the account and issues a different code.
    let (_, unpaired) = call(&router, "POST", "/api/telegram/unpair", Some(&secret), None).await;
    assert_eq!(unpaired["paired"], false);
    assert!(unpaired["pairCode"].as_str().is_some_and(|c| c.len() == 6));

    let _ = std::fs::remove_dir_all(&home);
}
