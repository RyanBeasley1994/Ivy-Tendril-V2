//! Talking to the managers from Telegram.
//!
//! One bot, one person. The operator creates a bot with BotFather, gives the daemon its token in
//! Settings, and pairs their own Telegram account by sending the bot a one-time code. From then on the
//! bot answers that account, in that private chat, and nobody else: an update from any other chat or
//! user is dropped without a reply, so the bot cannot be used by someone who merely finds it.
//!
//! * `/manager <name>` picks a project (the name need not be exact) and binds the chat to its manager.
//!   Until `/end`, every message typed is handed to that manager exactly as typing it in the app would,
//!   and everything said in that manager's chat comes back.
//! * A manager that needs the operator, or whose work is complete, sends a notice here whether or not
//!   the chat is bound, with the pull request's link when it names one. Replying to a notice answers
//!   that manager, and binds the chat to it so the conversation carries on.
//! * `/status` is answered by the daemon from what it already knows, so it costs no manager turn.
//!
//! The daemon polls Telegram (`getUpdates`), so it needs no public address and no webhook.

use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tendril_core::chat::execution::{ChatEvent, ChatTurnOptions};
use tendril_core::chat::manager_brief::{is_briefing, manager_session_id};
use tendril_core::chat::models::ChatQueuedItem;
use tendril_core::config::load_config;
use tokio::sync::Mutex;

/// Telegram refuses a message longer than 4096 characters.
const MAX_MESSAGE_CHARS: usize = 3900;
/// How much of a daemon event is forwarded: enough to say what it was.
const EVENT_CHARS: usize = 300;
/// Notices remembered so a reply to one can be routed to its manager.
const MAX_NOTICES: usize = 50;
/// Seconds one `getUpdates` call waits for a message before returning empty.
const POLL_SECONDS: u64 = 25;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The bot's token from BotFather. Never sent back to the app.
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub bot_username: String,
    /// What the operator sends the bot (`/start <code>`) to pair their account.
    #[serde(default)]
    pub pair_code: String,
    /// The private chat and the account the bot answers. Both must match on every update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    /// The project whose manager the chat is talking to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound: Option<String>,
    /// The next update to ask Telegram for.
    #[serde(default)]
    pub offset: i64,
    /// Notices sent, newest last: the Telegram message id and the project it was about.
    #[serde(default)]
    pub notices: Vec<(i64, String)>,
}

impl Settings {
    pub fn paired(&self) -> bool {
        !self.token.is_empty() && self.chat_id.is_some() && self.user_id.is_some()
    }

    /// Whether an update came from the paired account in the paired chat.
    pub fn is_operator(&self, chat_id: i64, user_id: i64) -> bool {
        self.chat_id == Some(chat_id) && self.user_id == Some(user_id)
    }
}

static FILE_LOCK: Mutex<()> = Mutex::const_new(());

fn path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("telegram.json")
}

pub fn load(tendril_home: &Path) -> Settings {
    std::fs::read_to_string(path(tendril_home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Reads the settings, changes them, and writes them back, one writer at a time.
async fn change(tendril_home: &Path, edit: impl FnOnce(&mut Settings)) -> Settings {
    let _guard = FILE_LOCK.lock().await;
    let mut settings = load(tendril_home);
    edit(&mut settings);
    let file = path(tendril_home);
    if let Ok(bytes) = serde_json::to_vec_pretty(&settings) {
        if std::fs::write(&file, bytes).is_ok() {
            // It holds the bot's token: the daemon's user only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    settings
}

fn new_pair_code() -> String {
    format!("{:06}", uuid::Uuid::new_v4().as_u128() % 1_000_000)
}

// ---- the Telegram API ----------------------------------------------------------------------------

fn api_base() -> String {
    std::env::var("TENDRIL_TELEGRAM_API").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "https://api.telegram.org".into())
}

async fn call(token: &str, method: &str, body: Value, timeout: Duration) -> Result<Value, String> {
    let client = reqwest::Client::builder().timeout(timeout).build().map_err(|e| e.to_string())?;
    let url = format!("{}/bot{token}/{method}", api_base().trim_end_matches('/'));
    // The token is in the URL, so an error must never carry the URL into a log or a reply.
    let resp = client.post(&url).json(&body).send().await.map_err(|e| e.without_url().to_string())?;
    let value: Value = resp.json().await.map_err(|e| e.without_url().to_string())?;
    if value["ok"].as_bool() == Some(true) {
        Ok(value["result"].clone())
    } else {
        Err(value["description"].as_str().unwrap_or("Telegram refused the request").to_string())
    }
}

/// A long message as pieces Telegram will take, cut at line ends where it can.
pub fn split_message(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.trim().split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current).trim_end().to_string());
        }
        let mut rest: Vec<char> = line.chars().collect();
        while rest.len() > max {
            out.push(rest.drain(..max).collect());
        }
        current.extend(rest);
    }
    if !current.trim().is_empty() {
        out.push(current.trim_end().to_string());
    }
    out
}

/// Sends text to the paired chat. Returns the id of the last message sent.
async fn send(settings: &Settings, text: &str) -> Option<i64> {
    let chat_id = settings.chat_id.filter(|_| settings.paired())?;
    send_to(&settings.token, chat_id, text).await
}

async fn send_to(token: &str, chat_id: i64, text: &str) -> Option<i64> {
    let mut last = None;
    for piece in split_message(text, MAX_MESSAGE_CHARS) {
        let body = json!({ "chat_id": chat_id, "text": piece, "disable_web_page_preview": true });
        match call(token, "sendMessage", body, Duration::from_secs(15)).await {
            Ok(result) => last = result["message_id"].as_i64(),
            Err(e) => tracing::warn!("Telegram: could not send a message: {e}"),
        }
    }
    last
}

// ---- what the operator typed ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `/start <code>`: pair this account.
    Start(String),
    /// `/manager <name>`: talk to that project's manager. Empty lists the projects.
    Manager(String),
    End,
    Status,
    Help,
    /// Anything that is not a command: for the manager.
    Text(String),
}

pub fn parse(text: &str) -> Command {
    let text = text.trim();
    let Some(rest) = text.strip_prefix('/') else { return Command::Text(text.to_string()) };
    let (word, argument) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    // In a group Telegram writes `/manager@BotName`; the bot only works in a private chat, but a
    // command copied from one should still read.
    let word = word.split('@').next().unwrap_or(word).to_ascii_lowercase();
    let argument = argument.trim().to_string();
    match word.as_str() {
        "start" => Command::Start(argument),
        "manager" | "m" | "project" => Command::Manager(argument),
        "end" | "stop" | "exit" | "bye" => Command::End,
        "status" | "managers" | "projects" => Command::Status,
        "help" => Command::Help,
        // An unknown `/word` is far more likely a message that starts with a path than a command.
        _ => Command::Text(text.to_string()),
    }
}

fn letters(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|n| chars.any(|h| h == n))
}

/// The projects a loosely typed name could mean, best first. One entry means it is unambiguous.
///
/// Punctuation, spacing and case are ignored, so "ypf direct" finds "YPF-Direct". A name that contains
/// what was typed beats one that merely has its letters in order, and when exactly one contains it,
/// that one alone is returned.
pub fn match_projects(query: &str, names: &[String]) -> Vec<String> {
    let q = letters(query);
    if q.is_empty() {
        return Vec::new();
    }
    if let Some(exact) = names.iter().find(|n| letters(n) == q) {
        return vec![exact.clone()];
    }
    let mut containing: Vec<&String> = names.iter().filter(|n| letters(n).contains(&q)).collect();
    if !containing.is_empty() {
        // A name that starts with it, then the shorter name.
        containing.sort_by_key(|n| (!letters(n).starts_with(&q), n.len()));
        return containing.into_iter().cloned().collect();
    }
    let mut loose: Vec<&String> = names.iter().filter(|n| is_subsequence(&q, &letters(n))).collect();
    loose.sort_by_key(|n| n.len());
    loose.into_iter().cloned().collect()
}

/// The pull request numbers a message names ("PR 46", "pull request #12"), in order, without repeats.
pub fn pr_numbers(text: &str) -> Vec<u64> {
    let Ok(re) = regex::Regex::new(r"(?i)\b(?:PR|pull request)\s*#?(\d{1,6})\b") else { return Vec::new() };
    let mut out: Vec<u64> = Vec::new();
    for cap in re.captures_iter(text) {
        if let Ok(n) = cap[1].parse::<u64>() {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out.truncate(3);
    out
}

const HELP: &str = "/manager <project> - talk to that project's manager (the name need not be exact)\n\
/end - stop talking to it\n\
/status - what every manager is doing\n\
While you are talking to a manager, everything you type goes to it and everything in its chat comes back. \
A manager that needs you messages you here; reply to that message to answer it.";

// ---- acting on it --------------------------------------------------------------------------------

fn project_names(state: &AppState) -> Vec<String> {
    load_config(&state.config_path).map(|s| s.projects.into_iter().map(|p| p.name).collect()).unwrap_or_default()
}

/// Hands text to a project's manager the way the app does. Returns whether it was queued behind a
/// turn already running.
async fn tell_manager(state: &Arc<AppState>, project: &str, text: &str) -> Result<bool, String> {
    let opened = crate::routes::projects::get_or_create_project_manager(
        State(state.clone()),
        axum::extract::Path(project.to_string()),
    )
    .await
    .into_response();
    if !opened.status().is_success() {
        return Err(format!("Could not open the manager of {project}."));
    }
    let id = manager_session_id(project);
    if state.chat_manager.is_generating(&id).await {
        let item = ChatQueuedItem {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: text.to_string(),
            attachments: None,
            created_at: chrono::Utc::now(),
            role: None,
        };
        state.chat_manager.enqueue_message(&id, item).await;
        return Ok(true);
    }
    state.chat_manager.start_session_turn(&id, text, ChatTurnOptions::default()).await.map(|()| false).map_err(|e| e.to_string())
}

/// One line per project that has a manager: what it is doing and whether it needs the operator.
async fn status_text(state: &AppState, bound: Option<&str>) -> String {
    let mut lines = Vec::new();
    for project in project_names(state) {
        let id = manager_session_id(&project);
        if state.chat_manager.get_session(&id).await.is_err() {
            continue;
        }
        let working = state.chat_manager.is_generating(&id).await;
        let attention = crate::attention::for_project(state, &project).await;
        let tasks = crate::manager_tasks::list(&state.tendril_home, &project).iter().filter(|t| t.running()).count();
        let mut parts = vec![if working { "working".to_string() } else { "idle".to_string() }];
        if tasks > 0 {
            parts.push(format!("{tasks} task{} running", if tasks == 1 { "" } else { "s" }));
        }
        if attention.count() > 0 {
            parts.push(if attention.asked { "asked you something".to_string() } else { "needs you".to_string() });
        }
        let here = if bound.is_some_and(|b| b.eq_ignore_ascii_case(&project)) { " (talking to it)" } else { "" };
        lines.push(format!("{project}{here}: {}", parts.join(", ")));
    }
    if lines.is_empty() {
        "No project has a manager yet. Open one in the app first.".to_string()
    } else {
        lines.join("\n")
    }
}

/// What was typed from Telegram lately, so the forwarder does not send the operator their own words back.
type Echoes = Arc<std::sync::Mutex<VecDeque<String>>>;

async fn handle_text(state: &Arc<AppState>, settings: &Settings, text: &str, reply_to: Option<i64>, echoes: &Echoes) {
    let home = &state.tendril_home;
    match parse(text) {
        Command::Start(_) | Command::Help => {
            send(settings, HELP).await;
        }
        Command::Status => {
            let text = status_text(state, settings.bound.as_deref()).await;
            send(settings, &text).await;
        }
        Command::End => {
            let was = settings.bound.clone();
            change(home, |s| s.bound = None).await;
            let text = match was {
                Some(project) => format!("Stopped talking to {project}. It will still message you if it needs you."),
                None => "You were not talking to a manager.".to_string(),
            };
            send(settings, &text).await;
        }
        Command::Manager(name) => {
            let names = project_names(state);
            if name.is_empty() {
                send(settings, &format!("Which project? /manager <name>\n{}", names.join("\n"))).await;
                return;
            }
            let found = match_projects(&name, &names);
            match found.as_slice() {
                [] => {
                    send(settings, &format!("No project matches \"{name}\". You have:\n{}", names.join("\n"))).await;
                }
                [project] => {
                    change(home, |s| s.bound = Some(project.clone())).await;
                    send(settings, &format!("Talking to {project}'s manager. Everything you type goes to it. /end to stop.")).await;
                }
                several => {
                    let list = several.iter().map(|p| format!("/manager {p}")).collect::<Vec<_>>().join("\n");
                    send(settings, &format!("\"{name}\" could be more than one project:\n{list}")).await;
                }
            }
        }
        Command::Text(text) => {
            // A reply to a notice is for the manager the notice came from, and carries on from there.
            let from_notice = reply_to.and_then(|id| settings.notices.iter().find(|(m, _)| *m == id).map(|(_, p)| p.clone()));
            let Some(project) = from_notice.clone().or_else(|| settings.bound.clone()) else {
                send(settings, "You are not talking to a manager. /manager <project> to pick one, /status to see them.").await;
                return;
            };
            if from_notice.is_some() && settings.bound.as_deref() != Some(project.as_str()) {
                change(home, |s| s.bound = Some(project.clone())).await;
                send(settings, &format!("Now talking to {project}'s manager. /end to stop.")).await;
            }
            if let Ok(mut recent) = echoes.lock() {
                recent.push_back(text.clone());
                while recent.len() > 20 {
                    recent.pop_front();
                }
            }
            match tell_manager(state, &project, &text).await {
                Ok(true) => {
                    send(settings, "It is busy: your message is queued and it will take it up next.").await;
                }
                Ok(false) => {}
                Err(e) => {
                    send(settings, &format!("That did not reach {project}'s manager: {e}")).await;
                }
            }
        }
    }
}

/// One update from Telegram. Anything that is not the paired account in its private chat is dropped,
/// except the one message that pairs it.
async fn handle_update(state: &Arc<AppState>, update: &Value, echoes: &Echoes) {
    let message = &update["message"];
    let (Some(chat_id), Some(user_id), Some(text)) =
        (message["chat"]["id"].as_i64(), message["from"]["id"].as_i64(), message["text"].as_str())
    else {
        return;
    };
    let settings = load(&state.tendril_home);
    if settings.is_operator(chat_id, user_id) {
        handle_text(state, &settings, text, message["reply_to_message"]["message_id"].as_i64(), echoes).await;
        return;
    }
    // Pairing: only before anyone is paired, only in a private chat, only with the code Settings shows.
    let private = message["chat"]["type"].as_str() == Some("private");
    let pairs = matches!(parse(text), Command::Start(code) if !settings.pair_code.is_empty() && code == settings.pair_code);
    if settings.chat_id.is_none() && private && pairs {
        let name = [message["from"]["first_name"].as_str(), message["from"]["username"].as_str().map(|u| u.trim())]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let name = match name.as_slice() {
            [first, user] => format!("{first} (@{user})"),
            [one] => (*one).to_string(),
            _ => format!("user {user_id}"),
        };
        let paired = change(&state.tendril_home, |s| {
            s.chat_id = Some(chat_id);
            s.user_id = Some(user_id);
            s.user_name = Some(name);
            s.pair_code.clear();
        })
        .await;
        tracing::info!("Telegram: paired with {}", paired.user_name.as_deref().unwrap_or("an account"));
        send(&paired, &format!("Paired. This bot now answers only you.\n\n{HELP}")).await;
    }
    // Everyone else gets silence: a reply would tell a stranger the bot is alive.
}

// ---- what the managers say -----------------------------------------------------------------------

/// The links of the pull requests a manager's message names, so "PR 46 is green" can be tapped.
async fn pr_links(state: &AppState, project: &str, text: &str) -> Vec<String> {
    let numbers = pr_numbers(text);
    if numbers.is_empty() {
        return Vec::new();
    }
    let Some(dir) = crate::manager_scheduler::project_repo_dir(state, project) else { return Vec::new() };
    let mut links = Vec::new();
    for n in numbers {
        let mut cmd = tokio::process::Command::new("gh");
        cmd.current_dir(&dir).args(["pr", "view", &n.to_string(), "--json", "url", "-q", ".url"]);
        if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await {
            let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && url.starts_with("http") && !text.contains(&url) {
                links.push(url);
            }
        }
    }
    links
}

fn with_links(text: &str, links: &[String]) -> String {
    if links.is_empty() {
        text.to_string()
    } else {
        format!("{text}\n\n{}", links.join("\n"))
    }
}

/// Whether Telegram is set up and paired, so the scheduler knows a notice has somewhere to go.
pub fn enabled(tendril_home: &Path) -> bool {
    load(tendril_home).paired()
}

/// A manager needs the operator, or its work is complete. Not sent while the chat is bound to that
/// manager: its reply has already arrived as part of the conversation.
pub async fn notify(state: &AppState, project: &str, title: &str, body: &str) {
    let settings = load(&state.tendril_home);
    if !settings.paired() || settings.bound.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(project)) {
        return;
    }
    let links = pr_links(state, project, body).await;
    let text = with_links(&format!("{title}\n{body}\n\nReply to this message to answer {project}'s manager."), &links);
    if let Some(message_id) = send(&settings, &text).await {
        let project = project.to_string();
        change(&state.tendril_home, |s| {
            s.notices.push((message_id, project));
            let extra = s.notices.len().saturating_sub(MAX_NOTICES);
            s.notices.drain(..extra);
        })
        .await;
    }
}

/// How a message in the bound manager's chat reads in Telegram, or `None` for one not worth sending.
pub fn forwarded(role: &str, content: &str, typed_here: bool) -> Option<String> {
    let content = content.trim();
    if content.is_empty() {
        return None;
    }
    match role {
        "assistant" => Some(content.to_string()),
        // Their own words came from this chat: sending them back is noise. Typed in the app, they are
        // part of the conversation being followed.
        "user" if typed_here => None,
        "user" => Some(format!("You, in the app:\n{content}")),
        "system" if is_briefing(content) || content.starts_with("Engine switched by the operator") => None,
        "system" => Some(format!("[event] {}", crate::push::clip(&crate::push::plain(content), EVENT_CHARS))),
        _ => None,
    }
}

/// Sends everything said in the bound manager's chat to Telegram, as it is said.
async fn forward_chat(state: Arc<AppState>, echoes: Echoes) {
    let mut events = state.chat_manager.subscribe_events();
    let mut sent: HashSet<String> = HashSet::new();
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        let ChatEvent::MessageAdded { session_id, message } = event else { continue };
        let settings = load(&state.tendril_home);
        let Some(project) = settings.bound.clone().filter(|_| settings.paired()) else { continue };
        if session_id != manager_session_id(&project) {
            continue;
        }
        let typed_here = message.role == "user"
            && echoes.lock().map(|mut recent| match recent.iter().position(|t| t == message.content.trim()) {
                Some(i) => recent.remove(i).is_some(),
                None => false,
            })
            .unwrap_or(false);
        // A reply is announced twice, as an empty stub when its turn starts and whole when it ends;
        // only the whole one has anything to send, and it is sent once.
        let Some(text) = forwarded(&message.role, &message.content, typed_here) else { continue };
        if !sent.insert(message.id.clone()) {
            continue;
        }
        if sent.len() > 2000 {
            sent.clear();
        }
        let links = if message.role == "assistant" { pr_links(&state, &project, &text).await } else { Vec::new() };
        send(&settings, &with_links(&text, &links)).await;
    }
}

async fn poll(state: Arc<AppState>, echoes: Echoes) {
    loop {
        let settings = load(&state.tendril_home);
        if settings.token.is_empty() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        let body = json!({ "offset": settings.offset, "timeout": POLL_SECONDS, "allowed_updates": ["message"] });
        match call(&settings.token, "getUpdates", body, Duration::from_secs(POLL_SECONDS + 15)).await {
            Ok(updates) => {
                for update in updates.as_array().cloned().unwrap_or_default() {
                    // Moved on before it is handled: an update that makes the daemon fall over must
                    // not be the first thing it is handed when it comes back.
                    if let Some(id) = update["update_id"].as_i64() {
                        change(&state.tendril_home, |s| s.offset = s.offset.max(id + 1)).await;
                    }
                    handle_update(&state, &update, &echoes).await;
                }
            }
            Err(e) => {
                tracing::debug!("Telegram: getUpdates failed: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

pub fn spawn(state: Arc<AppState>) {
    let echoes: Echoes = Arc::default();
    tokio::spawn(poll(state.clone(), echoes.clone()));
    tokio::spawn(forward_chat(state, echoes));
}

// ---- Settings ------------------------------------------------------------------------------------

fn status_json(settings: &Settings) -> Value {
    json!({
        "configured": !settings.token.is_empty(),
        "botUsername": settings.bot_username,
        "paired": settings.paired(),
        "pairedWith": settings.user_name,
        // Shown only until someone pairs; there is nothing to pair with afterwards.
        "pairCode": if settings.paired() { Value::Null } else { json!(settings.pair_code) },
        "talkingTo": settings.bound,
    })
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// `GET /api/telegram`: whether a bot is set, and who it is paired with. Never the token.
pub async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(status_json(&load(&state.tendril_home)))
}

#[derive(Debug, Deserialize)]
pub struct SetBody {
    pub token: String,
}

/// `PUT /api/telegram`: sets the bot. The token is checked with Telegram first; a new bot starts
/// unpaired, with a fresh code to pair with.
pub async fn set_handler(State(state): State<Arc<AppState>>, Json(body): Json<SetBody>) -> Response {
    let token = body.token.trim().to_string();
    if token.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Paste the token BotFather gave you.");
    }
    let me = match call(&token, "getMe", json!({}), Duration::from_secs(15)).await {
        Ok(me) => me,
        Err(e) => return error(StatusCode::BAD_REQUEST, format!("Telegram did not accept that token: {e}")),
    };
    let username = me["username"].as_str().unwrap_or_default().to_string();
    let settings = change(&state.tendril_home, |s| {
        *s = Settings { token, bot_username: username, pair_code: new_pair_code(), ..Settings::default() };
    })
    .await;
    Json(status_json(&settings)).into_response()
}

/// `POST /api/telegram/unpair`: forgets the paired account and issues a new code. The bot stays.
pub async fn unpair_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = change(&state.tendril_home, |s| {
        s.chat_id = None;
        s.user_id = None;
        s.user_name = None;
        s.bound = None;
        s.notices.clear();
        s.pair_code = new_pair_code();
    })
    .await;
    Json(status_json(&settings))
}

/// `POST /api/telegram/test`: sends the paired account a message, to see that it arrives.
pub async fn test_handler(State(state): State<Arc<AppState>>) -> Response {
    let settings = load(&state.tendril_home);
    if !settings.paired() {
        return error(StatusCode::BAD_REQUEST, "Pair your Telegram account first.");
    }
    match send(&settings, "Tendril can reach you here. /status shows what your managers are doing.").await {
        Some(_) => Json(json!({ "sent": true })).into_response(),
        None => error(StatusCode::BAD_GATEWAY, "Telegram did not take the message. Have you blocked the bot?"),
    }
}

/// `DELETE /api/telegram`: removes the bot and everything known about it.
pub async fn remove_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = change(&state.tendril_home, |s| *s = Settings::default()).await;
    Json(status_json(&settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["YPF-Direct", "Monorepo-Propfirm", "Propfirm Docs", "Trading Platform"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn commands_are_read_and_anything_else_is_for_the_manager() {
        assert_eq!(parse("/manager ypf direct"), Command::Manager("ypf direct".into()));
        assert_eq!(parse("  /Manager@ForgeBot   YPF "), Command::Manager("YPF".into()));
        assert_eq!(parse("/manager"), Command::Manager(String::new()));
        assert_eq!(parse("/start 123456"), Command::Start("123456".into()));
        assert_eq!(parse("/end"), Command::End);
        assert_eq!(parse("/status"), Command::Status);
        assert_eq!(parse("merge it"), Command::Text("merge it".into()));
        // A message that merely begins with a slash is not swallowed.
        assert_eq!(parse("/api/users is returning 500"), Command::Text("/api/users is returning 500".into()));
    }

    #[test]
    fn a_project_is_found_by_a_loose_name() {
        let names = names();
        assert_eq!(match_projects("ypf", &names), vec!["YPF-Direct"]);
        assert_eq!(match_projects("ypf direct", &names), vec!["YPF-Direct"]);
        assert_eq!(match_projects("TRADING", &names), vec!["Trading Platform"]);
        assert_eq!(match_projects("monorepo-propfirm", &names), vec!["Monorepo-Propfirm"], "an exact name wins outright");
        // Two contain it: both are offered, the one that starts with it first.
        assert_eq!(match_projects("propfirm", &names), vec!["Propfirm Docs", "Monorepo-Propfirm"]);
        // Letters in order, when nothing contains it.
        assert_eq!(match_projects("trdplat", &names), vec!["Trading Platform"]);
        assert!(match_projects("zebra", &names).is_empty());
        assert!(match_projects("  ", &names).is_empty());
    }

    #[test]
    fn only_the_paired_account_in_its_own_chat_is_the_operator() {
        let s = Settings { token: "t".into(), chat_id: Some(10), user_id: Some(7), ..Settings::default() };
        assert!(s.paired() && s.is_operator(10, 7));
        assert!(!s.is_operator(10, 8), "another person in the same chat");
        assert!(!s.is_operator(11, 7), "the same person somewhere else");
        let unpaired = Settings { token: "t".into(), pair_code: "123456".into(), ..Settings::default() };
        assert!(!unpaired.paired() && !unpaired.is_operator(10, 7));
        // The app is never shown the token, and the code only until someone pairs.
        let shown = status_json(&unpaired);
        assert!(shown.get("token").is_none() && shown["pairCode"] == "123456");
        assert!(status_json(&s)["pairCode"].is_null());
    }

    #[test]
    fn the_whole_chat_is_forwarded_except_what_they_typed_here_and_the_briefing() {
        assert_eq!(forwarded("assistant", " PR 46 is green. ", false).as_deref(), Some("PR 46 is green."));
        assert_eq!(forwarded("assistant", "   ", false), None, "the stub of a turn that has not answered yet");
        assert_eq!(forwarded("user", "merge it", true), None);
        assert_eq!(forwarded("user", "merge it", false).as_deref(), Some("You, in the app:\nmerge it"));
        assert_eq!(forwarded("system", "# You are the Factory Manager for project \"x\"", false), None);
        let event = forwarded("system", &format!("Task ab12 has stopped. {}", "x".repeat(900)), false).unwrap();
        assert!(event.starts_with("[event] Task ab12 has stopped.") && event.chars().count() < 320, "{event}");
    }

    #[test]
    fn pull_requests_named_in_a_message_are_found() {
        assert_eq!(pr_numbers("PR 46 is green and ready to merge; pull request #12 is still running. PR 46 again."), vec![46, 12]);
        assert!(pr_numbers("Nothing to ship yet. The APR is 4%.").is_empty());
    }

    #[test]
    fn a_long_message_is_cut_at_line_ends() {
        let text = (1..=10).map(|i| format!("line {i} {}", "x".repeat(40))).collect::<Vec<_>>().join("\n");
        let pieces = split_message(&text, 120);
        assert!(pieces.len() > 3 && pieces.iter().all(|p| p.chars().count() <= 120), "{pieces:?}");
        assert!(pieces[0].starts_with("line 1 ") && pieces[0].ends_with('x'));
        assert_eq!(pieces.join("\n"), text);
        // One enormous line is still cut.
        assert_eq!(split_message(&"y".repeat(250), 100).len(), 3);
        assert_eq!(split_message("short", 100), vec!["short"]);
    }

    #[test]
    fn a_pair_code_is_six_digits() {
        let code = new_pair_code();
        assert!(code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()), "{code}");
    }
}
