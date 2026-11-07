//! Support tickets from Propriotec Operations, worked from Tendril.
//!
//! Operations is where partners raise tickets (portal, Slack, Discord). Its own UI is a Kanban board
//! over `/api/team/tickets`; this is the same board's client, so the Support page can read, reply,
//! move and close tickets, and hand one to a project's manager to fix.
//!
//! **Signed in as the operator.** Operations' bearer-key `v1` API has no notion of *who* is reading,
//! so it cannot keep the operator's read receipts or clear their notifications. The team routes can,
//! so the daemon signs in with the operator's own Operations login and calls exactly what the
//! Operations UI calls: opening a ticket posts `/read` and `/api/notifications/read-by-ticket`, which
//! is what clears the badge there and here.
//!
//! **Storage.** `<TendrilHome>/Support.json` holds the URL, the login and the current session token,
//! written owner-only. The password never leaves the daemon: [`SupportStatus`] is what the UI sees.
//! `<TendrilHome>/support-handoffs.json` records which tickets were handed to which project.
//!
//! **Hidden from the customer.** Handing a ticket to a project starts a fresh manager conversation
//! about it (`manager_brief::ticket_session_id`), shown beside the ticket, where the operator guides
//! the work. Nothing from it reaches the partner until `tendril support close --message`.

use crate::error::{Result, TendrilError};
use crate::fs_lock::{with_file_lock, write_atomic};
use crate::support_cases;
use chrono::{DateTime, Utc};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://portal.propriotec.com";
/// Operations' `SESSION_COOKIE_CONFIG.name`.
const SESSION_COOKIE: &str = "cc_session";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Statuses the Operations board moves a ticket between.
pub const STATUSES: &[&str] = &["PENDING", "OPEN", "IN_PROGRESS", "AWAITING_REPLY", "RESOLVED", "CLOSED"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SupportConnection {
    pub base_url: String,
    pub email: String,
    pub password: String,
    /// The session the last sign-in returned; replaced on a 401.
    pub token: Option<String>,
    /// Who that sign-in was, as Operations reported it.
    pub user_name: Option<String>,
}

/// What the UI is told about the connection: never the password or the token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportStatus {
    pub configured: bool,
    pub base_url: String,
    pub email: String,
    pub user_name: Option<String>,
}

fn connection_path(home: &Path) -> PathBuf {
    home.join("Support.json")
}

fn handoffs_path(home: &Path) -> PathBuf {
    home.join("support-handoffs.json")
}

pub fn load_connection(home: &Path) -> Option<SupportConnection> {
    let text = std::fs::read_to_string(connection_path(home)).ok()?;
    let conn: SupportConnection = serde_json::from_str(&text).ok()?;
    (!conn.email.is_empty() && !conn.password.is_empty()).then_some(conn)
}

fn save_connection(home: &Path, conn: &SupportConnection) -> Result<()> {
    let path = connection_path(home);
    let json = serde_json::to_vec_pretty(conn).map_err(|e| TendrilError::Other(e.to_string()))?;
    write_atomic(&path, &json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn status(home: &Path) -> SupportStatus {
    match load_connection(home) {
        Some(c) => SupportStatus { configured: true, base_url: c.base_url, email: c.email, user_name: c.user_name },
        None => SupportStatus { configured: false, base_url: DEFAULT_BASE_URL.into(), email: String::new(), user_name: None },
    }
}

pub fn disconnect(home: &Path) -> Result<()> {
    match std::fs::remove_file(connection_path(home)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn normalize_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() { DEFAULT_BASE_URL.into() } else { url.into() }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default()
}

/// Signs in and, when it works, saves the connection. A wrong password is refused here rather than
/// saved and failing later.
pub async fn connect(home: &Path, base_url: &str, email: &str, password: &str) -> Result<SupportStatus> {
    let mut conn = SupportConnection {
        base_url: normalize_base(base_url),
        email: email.trim().to_string(),
        password: password.to_string(),
        token: None,
        user_name: None,
    };
    login(&mut conn).await?;
    save_connection(home, &conn)?;
    Ok(status(home))
}

async fn login(conn: &mut SupportConnection) -> Result<()> {
    let resp = http()
        .post(format!("{}/api/auth/login", conn.base_url))
        .json(&json!({ "email": conn.email, "password": conn.password }))
        .send()
        .await
        .map_err(|e| TendrilError::Other(format!("Could not reach Operations at {}: {e}", conn.base_url)))?;
    let code = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !code.is_success() {
        let reason = body["error"].as_str().unwrap_or("sign-in failed");
        return Err(TendrilError::Validation(format!("Operations refused the sign-in: {reason}")));
    }
    let token = body["token"]
        .as_str()
        .ok_or_else(|| TendrilError::Other("Operations answered the sign-in without a session".into()))?;
    let role = body["user"]["role"].as_str().unwrap_or_default();
    if role != "ADMIN" && role != "SUPPORT" {
        return Err(TendrilError::Validation(format!(
            "That Operations account is {role}, and only ADMIN or SUPPORT users can work tickets."
        )));
    }
    conn.token = Some(token.to_string());
    conn.user_name = body["user"]["name"].as_str().map(str::to_string);
    Ok(())
}

/// One call against Operations as the signed-in operator, signing in again once if the session
/// lapsed.
pub async fn request(home: &Path, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
    // A dry run on a ticket reads it but changes nothing there: no replies, notes, statuses or types.
    if method != Method::GET && !path.ends_with("/read") {
        if let Some(ticket) = path.strip_prefix("/api/team/tickets/").and_then(|rest| rest.split('/').next()) {
            if support_cases::is_dry(home, ticket) {
                return Ok(json!({ "dryRun": true }));
            }
        }
    }
    let mut conn = load_connection(home)
        .ok_or_else(|| TendrilError::Validation("Support is not connected. Sign in to Operations on the Support page first.".into()))?;
    if conn.token.is_none() {
        login(&mut conn).await?;
        save_connection(home, &conn)?;
    }
    for attempt in 0..2 {
        let mut req = http()
            .request(method.clone(), format!("{}{}", conn.base_url, path))
            .header("Cookie", format!("{SESSION_COOKIE}={}", conn.token.as_deref().unwrap_or_default()));
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| TendrilError::Other(format!("Could not reach Operations: {e}")))?;
        let code = resp.status();
        if code == StatusCode::UNAUTHORIZED && attempt == 0 {
            login(&mut conn).await?;
            save_connection(home, &conn)?;
            continue;
        }
        let value: Value = resp.json().await.unwrap_or(Value::Null);
        if !code.is_success() {
            let reason = value["error"].as_str().map(str::to_string).unwrap_or_else(|| code.to_string());
            return Err(TendrilError::Other(format!("Operations: {reason}")));
        }
        return Ok(value);
    }
    Err(TendrilError::Other("Operations kept refusing the session".into()))
}

/// A ticket id as one path segment. Ids are cuids, so this only ever matters for a malformed one.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The board: every ticket Operations' own board loads, with Tendril's handoffs attached.
pub async fn list_tickets(home: &Path, include_done: bool) -> Result<Value> {
    let mut query = String::from("limit=100");
    if !include_done {
        query.push_str("&excludeClosed=true");
    }
    let mut value = request(home, Method::GET, &format!("/api/team/tickets?{query}"), None).await?;
    let handoffs = load_handoffs(home);
    if let Some(tickets) = value["tickets"].as_array_mut() {
        for t in tickets {
            if let Some(h) = t["id"].as_str().and_then(|id| handoffs.iter().find(|h| h.ticket_id == id)) {
                t["handoff"] = json!(h);
            }
            if let Some(id) = t["id"].as_str().map(str::to_string) {
                t["triage"] = triage_summary(&support_cases::load_case(home, &id));
            }
        }
    }
    Ok(value)
}

/// The sidebar badge: the operator's unread tickets in "Needs Us", as Operations counts them.
pub async fn unread_count(home: &Path) -> Result<u64> {
    let v = request(home, Method::GET, "/api/team/tickets/count", None).await?;
    Ok(v["count"].as_u64().unwrap_or(0))
}

/// What the board shows of a ticket's case, or null when it has none.
fn triage_summary(case: &support_cases::Case) -> Value {
    if case.triaged_at.is_none() && case.investigation.is_none() && case.drafts.is_empty() {
        return Value::Null;
    }
    json!({
        "category": case.investigation.as_ref().map(|i| i.category),
        "severity": case.investigation.as_ref().map(|i| i.severity),
        "confidence": case.investigation.as_ref().map(|i| i.confidence),
        "pendingDrafts": case.pending_drafts(),
        "auto": case.triaged_at.is_some(),
        "working": case.working(),
    })
}

pub async fn get_ticket(home: &Path, id: &str) -> Result<Value> {
    let mut value = request(home, Method::GET, &format!("/api/team/tickets/{}", enc(id)), None).await?;
    if let Some(h) = load_handoffs(home).into_iter().find(|h| h.ticket_id == id) {
        value["handoff"] = json!(h);
    }
    Ok(value)
}

/// What the Operations ticket page does when it opens: a read receipt for this operator, and their
/// notifications for the ticket marked read.
pub async fn mark_read(home: &Path, id: &str) -> Result<()> {
    request(home, Method::POST, &format!("/api/team/tickets/{}/read", enc(id)), None).await?;
    request(home, Method::POST, "/api/notifications/read-by-ticket", Some(json!({ "ticketId": id }))).await?;
    Ok(())
}

/// A reply to the partner, or with `internal` a note only the team sees.
pub async fn reply(home: &Path, id: &str, content: &str, internal: bool) -> Result<Value> {
    if content.trim().is_empty() {
        return Err(TendrilError::Validation("A reply needs some text".into()));
    }
    request(
        home,
        Method::POST,
        &format!("/api/team/tickets/{}/reply", enc(id)),
        Some(json!({ "content": content, "isInternal": internal })),
    )
    .await
}

pub async fn set_status(home: &Path, id: &str, status: &str) -> Result<Value> {
    let status = status.trim().to_ascii_uppercase();
    if !STATUSES.contains(&status.as_str()) {
        return Err(TendrilError::Validation(format!("'{status}' is not a ticket status ({})", STATUSES.join(", "))));
    }
    if status == "RESOLVED" {
        return resolve(home, id).await;
    }
    request(home, Method::POST, &format!("/api/team/tickets/{}/status", enc(id)), Some(json!({ "status": status }))).await
}

/// Operations' ticket types, the columns of its By type board.
pub const TYPES: &[&str] = &["CRITICAL", "ISSUE", "REQUEST", "QUESTION"];

/// Sets a ticket's type. Operations applies its own rule on top: only CRITICAL carries CRITICAL
/// priority, so the answer has the type and priority it settled on.
pub async fn set_type(home: &Path, id: &str, ticket_type: &str) -> Result<Value> {
    let t = ticket_type.trim().to_ascii_uppercase();
    if !TYPES.contains(&t.as_str()) {
        return Err(TendrilError::Validation(format!("'{t}' is not a ticket type ({})", TYPES.join(", "))));
    }
    request(home, Method::POST, &format!("/api/team/tickets/{}/type", enc(id)), Some(json!({ "type": t }))).await
}

/// The Operations type an investigation puts a ticket in: Critical only for what Operations calls
/// critical (a platform-wide outage or a breach), else by category.
pub fn type_for(investigation: &support_cases::Investigation) -> &'static str {
    use support_cases::{Category, Severity};
    match (investigation.severity, investigation.category) {
        (Severity::Critical, _) => "CRITICAL",
        (_, Category::Bug) => "ISSUE",
        (_, Category::Question) => "QUESTION",
        (_, Category::Request) => "REQUEST",
    }
}

/// Operations' silent resolve: no message to the partner, notifications cleared. A ticket with a
/// triage case is written to the support history, so the next one like it starts from what was learnt.
pub async fn resolve(home: &Path, id: &str) -> Result<Value> {
    if support_cases::is_dry(home, id) {
        return Ok(json!({ "dryRun": true }));
    }
    let v = request(home, Method::POST, &format!("/api/team/tickets/{}/resolve", enc(id)), None).await?;
    mark_handoff_done(home, id)?;
    let case = support_cases::load_case(home, id);
    if case.investigation.is_some() || !case.drafts.is_empty() {
        let entry = support_cases::entry_from_case(home, &case, None, None);
        if let Err(e) = support_cases::remember(home, &entry) {
            tracing::warn!("Could not add ticket {id} to the support history: {e}");
        }
    }
    Ok(v)
}

/* ------------------------------------------------------------------ the manager's replies */

/// What became of a reply the manager asked to send.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum ReplyOutcome {
    /// Held for the operator's approval.
    Queued { draft: support_cases::Draft },
    Sent,
}

/// The ticket's own people: their addresses may be said back to them.
fn ticket_emails(ticket: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut take = |v: &Value| {
        if let Some(e) = v.as_str().filter(|e| e.contains('@')) {
            out.push(e.to_ascii_lowercase());
        }
    };
    take(&ticket["partner"]["email"]);
    take(&ticket["contactEmail"]);
    for v in ticket["intakeData"].as_object().into_iter().flat_map(|o| o.values()) {
        take(v);
    }
    for m in ticket["messages"].as_array().into_iter().flatten() {
        if m["authorType"].as_str() == Some("PARTNER") {
            take(&m["authorEmail"]);
        }
    }
    out
}

/// A fix PR as GitHub has it, through `gh`.
pub fn fix_pr_from_github(url: &str) -> Result<support_cases::FixPr> {
    let out = std::process::Command::new("gh")
        .args(["pr", "view", url, "--json", "state,additions,deletions,statusCheckRollup"])
        .output()
        .map_err(|e| TendrilError::Other(format!("Could not run gh: {e}")))?;
    if !out.status.success() {
        return Err(TendrilError::Other(format!("gh could not read {url}: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let checks = v["statusCheckRollup"].as_array().cloned().unwrap_or_default();
    let checks_green = checks.iter().all(|c| {
        let verdict = c["conclusion"].as_str().or(c["state"].as_str()).unwrap_or_default().to_ascii_uppercase();
        matches!(verdict.as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED")
    });
    Ok(support_cases::FixPr {
        state: v["state"].as_str().unwrap_or("UNKNOWN").to_string(),
        changed_lines: v["additions"].as_u64().unwrap_or(0) + v["deletions"].as_u64().unwrap_or(0),
        checks_green,
    })
}

/// A reply from the support manager: checked against the triage rules, then sent or held for the
/// operator as [`support_cases::SupportSettings::reply_mode`] says.
pub async fn agent_reply(home: &Path, id: &str, req: support_cases::ReplyRequest) -> Result<ReplyOutcome> {
    let ticket = get_ticket(home, id).await?;
    let case = support_cases::load_case(home, id);
    let fix_pr = |url: &str| fix_pr_from_github(url);
    support_cases::check_reply(&case, &req, &ticket_emails(&ticket), &fix_pr)?;
    let draft = support_cases::add_draft(home, id, &req)?;
    let settings = support_cases::load_settings(home);
    let held = settings.reply_mode == support_cases::ReplyMode::Approve && !settings.auto_kinds.contains(&req.kind);
    if support_cases::is_dry(home, id) || held {
        return Ok(ReplyOutcome::Queued { draft });
    }
    send_draft(home, id, &draft.id, None).await?;
    Ok(ReplyOutcome::Sent)
}

/// An internal note from the manager, through the note rules: one per change, written like a person.
/// A note hands the ticket to the operator, so it stops showing as being worked on.
pub async fn agent_note(home: &Path, id: &str, content: &str) -> Result<()> {
    let ticket = get_ticket(home, id).await?;
    let case = support_cases::load_case(home, id);
    let newest = ticket["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["createdAt"].as_str())
        .filter_map(|at| DateTime::parse_from_rfc3339(at).ok().map(|d| d.with_timezone(&Utc)))
        // Our own last note is on the ticket too; anything after it is what counts.
        .filter(|at| case.last_note_at.is_none_or(|last| *at > last + chrono::Duration::seconds(5)))
        .max();
    support_cases::check_note(&case, content, newest)?;
    if support_cases::is_dry(home, id) {
        support_cases::update_case(home, id, |c| {
            c.dry_notes.push(content.trim().to_string());
            c.last_note_at = Some(Utc::now());
            c.handed_over = true;
        })?;
        return Ok(());
    }
    reply(home, id, content.trim(), true).await?;
    support_cases::update_case(home, id, |c| {
        c.last_note_at = Some(Utc::now());
        c.handed_over = true;
    })?;
    Ok(())
}

/// Sends a held draft (the operator's approval, with their edit if they made one). A "resolved" reply
/// is also queued on every ticket following this one as a duplicate, each for its own approval.
pub async fn send_draft(home: &Path, id: &str, draft_id: &str, edited: Option<&str>) -> Result<support_cases::Draft> {
    if support_cases::is_dry(home, id) {
        return Err(TendrilError::Validation("This is a dry run: its replies are only shown, never sent".into()));
    }
    let pending = support_cases::load_case(home, id)
        .drafts
        .into_iter()
        .find(|d| d.id == draft_id && d.state == support_cases::DraftState::Pending)
        .ok_or_else(|| TendrilError::Validation("That draft is not waiting any more".into()))?;
    let content = edited.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&pending.content).to_string();
    reply(home, id, &content, false).await?;
    let draft = support_cases::decide_draft(home, id, draft_id, support_cases::DraftState::Sent, Some(&content))?;
    if draft.kind == support_cases::ReplyKind::Resolved {
        support_cases::update_case(home, id, |c| c.reopened = false)?;
    }
    if draft.kind == support_cases::ReplyKind::Resolved {
        for dup in support_cases::duplicates_of(home, id) {
            let req = support_cases::ReplyRequest {
                kind: support_cases::ReplyKind::Resolved,
                content: content.clone(),
                close: draft.close,
                fix_pr: draft.fix_pr.clone(),
                test: draft.test.clone(),
            };
            if let Err(e) = support_cases::add_draft(home, &dup.ticket_id, &req) {
                tracing::warn!("Could not queue the fix for duplicate ticket {}: {e}", dup.ticket_id);
            }
        }
    }
    if draft.close {
        resolve(home, id).await?;
    }
    Ok(draft)
}

pub fn discard_draft(home: &Path, id: &str, draft_id: &str) -> Result<support_cases::Draft> {
    support_cases::decide_draft(home, id, draft_id, support_cases::DraftState::Discarded, None)
}

/// Tells the partner `message` (when there is one), then resolves.
pub async fn close(home: &Path, id: &str, message: Option<&str>) -> Result<()> {
    if let Some(m) = message.map(str::trim).filter(|m| !m.is_empty()) {
        reply(home, id, m, false).await?;
    }
    resolve(home, id).await?;
    Ok(())
}

/* ------------------------------------------------------------------ handoffs */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Handoff {
    pub ticket_id: String,
    pub project: String,
    /// The ticket's manager conversation.
    #[serde(default)]
    pub session_id: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub done_at: Option<DateTime<Utc>>,
}

pub fn load_handoffs(home: &Path) -> Vec<Handoff> {
    std::fs::read_to_string(handoffs_path(home))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn update_handoffs(home: &Path, f: impl FnOnce(&mut Vec<Handoff>)) -> Result<()> {
    let path = handoffs_path(home);
    std::fs::create_dir_all(home)?;
    with_file_lock(&path, || {
        let mut all = load_handoffs(home);
        f(&mut all);
        let json = serde_json::to_vec_pretty(&all).map_err(|e| TendrilError::Other(e.to_string()))?;
        write_atomic(&path, &json)
    })
}

pub fn record_handoff(home: &Path, ticket_id: &str, project: &str, session_id: &str) -> Result<Handoff> {
    let handoff = Handoff {
        ticket_id: ticket_id.into(),
        project: project.into(),
        session_id: session_id.into(),
        at: Utc::now(),
        done_at: None,
    };
    let saved = handoff.clone();
    update_handoffs(home, |all| {
        all.retain(|h| h.ticket_id != ticket_id);
        all.push(saved);
    })?;
    Ok(handoff)
}

fn mark_handoff_done(home: &Path, ticket_id: &str) -> Result<()> {
    if !handoffs_path(home).exists() {
        return Ok(());
    }
    update_handoffs(home, |all| {
        for h in all.iter_mut().filter(|h| h.ticket_id == ticket_id && h.done_at.is_none()) {
            h.done_at = Some(Utc::now());
        }
    })
}

/// The first message of a ticket's manager conversation: the ticket as the team sees it, and the
/// rules for working it.
pub fn handoff_prompt(ticket: &Value, project: &str, instructions: &str) -> String {
    let id = ticket["id"].as_str().unwrap_or_default();
    let mut s = format!(
        "# Support ticket handed to you\nTicket `{id}` from Propriotec Operations: **{}**\nPartner: {} · status {} · priority {} · via {}\n",
        ticket["subject"].as_str().unwrap_or("(no subject)"),
        ticket["partner"]["name"].as_str().unwrap_or("unknown"),
        ticket["status"].as_str().unwrap_or("?"),
        ticket["priority"].as_str().unwrap_or("?"),
        ticket["platform"].as_str().unwrap_or("?"),
    );
    if let Some(intake) = ticket["intakeData"].as_object().filter(|o| !o.is_empty()) {
        s.push_str("Intake: ");
        let parts: Vec<String> = intake
            .iter()
            .filter_map(|(k, v)| v.as_str().filter(|v| !v.is_empty()).map(|v| format!("{k}: {v}")))
            .collect();
        s.push_str(&parts.join("; "));
        s.push('\n');
    }
    s.push_str("\n## Conversation\n");
    for m in ticket["messages"].as_array().into_iter().flatten() {
        let who = m["authorName"].as_str().unwrap_or("?");
        let kind = if m["isInternal"].as_bool().unwrap_or(false) { " (internal note)" } else { "" };
        let at = m["createdAt"].as_str().unwrap_or_default().get(..16).unwrap_or_default().replace('T', " ");
        s.push_str(&format!("- [{at}] {who}{kind}: {}\n", m["content"].as_str().unwrap_or_default().trim()));
    }
    if !instructions.trim().is_empty() {
        s.push_str(&format!("\n## From the operator\n{}\n", instructions.trim()));
    }
    s.push_str(&format!(
        r#"
## How to work it
This conversation is only about this ticket, for project **{project}**, and the operator is here guiding it.

**Investigate first, then stop and report.** Find out what is actually happening: have a worker read the code, reproduce what the partner described, check the data. Then tell the operator, briefly: what is going on, whether it is really a bug or the system working as designed, and what you recommend (fix it, reply with an explanation, or a policy decision for them). A ticket is a question from a partner, not an order to change anything.

**Change nothing until the operator says so.** No code changes, branches, pull requests, deploys or data fixes on your own initiative, even when you are sure. Wait for their go-ahead, then do exactly what they agreed, and take that to its finish line.

**The partner cannot see this conversation and must not hear from you unless the operator says so.** Report here, not on the ticket. Check for new partner messages with `tendril support get {id}`.

Every reply to the partner goes through the triage rules and, while approval is on, waits for the operator to approve it. First record your investigation (`tendril support investigate {id} ...`, see `tendril support investigate --help`), then reply with a kind: `tendril support reply {id} --kind ack|answer|resolved "<message>"`. When the operator says to reply and close (for example once an agreed fix is merged), do it in one step:
`tendril support close {id} --kind resolved --fix-pr <url> --test "<the test>" --message "<what they agreed to tell the partner>"`
Once it is settled, `tendril support learn {id} "<how it was fixed or settled>"` so the next ticket like it starts from that.
"#
    ));
    s
}

/// The first message of an automatically triaged ticket's conversation: the ticket, what similar
/// tickets taught, the open bugs it might duplicate, and the triage rules.
/// How far back "recently shipped" reaches.
const RECENT_HOURS: i64 = 48;

/// The pull requests merged into the project's repos in the last two days, one line each, newest first:
/// a regression report most often comes from one of them.
fn recent_changes(home: &Path, project: &str) -> String {
    let Ok(settings) = crate::config::load_config(&crate::config::get_config_path(home)) else {
        return String::new();
    };
    let Some(p) = settings.projects.iter().find(|p| p.name.eq_ignore_ascii_case(project)) else {
        return String::new();
    };
    let since = (Utc::now() - chrono::Duration::hours(RECENT_HOURS)).format("%Y-%m-%dT%H:%M:%SZ");
    let home_str = home.to_string_lossy();
    let mut merged: Vec<(String, String)> = Vec::new();
    for repo in &p.repos {
        let dir = crate::config::expand_variables(&repo.path, &home_str);
        let out = std::process::Command::new("gh")
            .args(["pr", "list", "--state", "merged", "--limit", "20", "--json", "number,title,mergedAt"])
            .args(["--search", &format!("merged:>={since}")])
            .current_dir(&dir)
            .output();
        let Some(out) = out.ok().filter(|o| o.status.success()) else { continue };
        let list: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap_or_default();
        for pr in list {
            let at = pr["mergedAt"].as_str().unwrap_or_default().to_string();
            let when = at.get(..16).unwrap_or_default().replace('T', " ");
            let name = std::path::Path::new(&dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let repo_note = if p.repos.len() > 1 { format!(" [{name}]") } else { String::new() };
            merged.push((at, format!("- #{} {}{repo_note} (merged {when} UTC)\n", pr["number"], pr["title"].as_str().unwrap_or_default())));
        }
    }
    merged.sort_by(|a, b| b.0.cmp(&a.0));
    merged.into_iter().map(|(_, line)| line).collect()
}

pub fn triage_prompt(home: &Path, ticket: &Value, project: &str) -> String {
    let id = ticket["id"].as_str().unwrap_or_default();
    let mut s = handoff_prompt(ticket, project, "");
    // The hand-off's working rules are for a ticket the operator brought; triage has its own.
    if let Some(cut) = s.find("\n## How to work it") {
        s.truncate(cut);
    }
    let subject = ticket["subject"].as_str().unwrap_or_default();
    let first = ticket["messages"][0]["content"].as_str().unwrap_or_default();
    let similar = support_cases::similar(home, &format!("{subject} {first}"), Some(id), 4);
    if !similar.is_empty() {
        s.push_str("\n## Similar tickets before\nHow these went: reuse what was found and how it was settled, and match the tone of what was said.\n");
        s.push_str(&support_cases::render_history(&similar));
    }
    let rewrites = support_cases::render_rewrites(home, 5);
    if !rewrites.is_empty() {
        s.push_str("\n## How the operator rewrites your replies\nThey edited these before sending. Write the way they do.\n");
        s.push_str(&rewrites);
    }
    let shipped = recent_changes(home, project);
    if !shipped.is_empty() {
        s.push_str("\n## Shipped in the last two days\nA problem that started recently is most often one of these. Check whether one touches what the partner reports (`gh pr view <number> --json files,body`), and if it does, cite it in --evidence.\n");
        s.push_str(&shipped);
    }
    let bugs = support_cases::open_bugs(home, id);
    if !bugs.is_empty() {
        s.push_str("\n## Bugs already being worked\nIf this ticket is the same problem, record it with `--duplicate-of <ticket>` and do not start a second fix.\n");
        for (t, subj, found) in bugs.iter().take(10) {
            s.push_str(&format!("- `{t}` {subj}: {found}\n"));
        }
    }
    let settings = support_cases::load_settings(home);
    let sources = settings.sources;
    s.push_str("\n## Data you can read\n");
    if settings.kubernetes {
        s.push_str(&format!(
            r#"- **The cluster**, with the server's kubeconfig. Find the service, its config and the Secrets it uses, then read through them; a Secret's values are used for you and never shown:
  - `tendril support kube --ticket {id} get deploy,svc,pods -n <ns>`, `describe deploy/<name> -n <ns>` (env and the Secrets it references), `logs deploy/<name> -n <ns> --since=2h`, `get events -n <ns>`
  - `tendril support kube-secret <ns>/<secret> --ticket {id}`: the keys a Secret holds
  - `tendril support kube-db <ns>/<secret>:<key> --ticket {id} find <collection> --filter '<json>' ...` (also count, aggregate, collections): the database whose connection string is in that key, port-forwarded for you
  - `tendril support kube-api <ns>/<service>:<port> <path> --ticket {id} [--bearer <secret>:<key>]`: a GET to an in-cluster API
  Reads only: changing anything in the cluster is refused, and so is exec.
"#
        ));
    }
    if sources.is_empty() && !settings.kubernetes {
        s.push_str("No data sources are configured; read the code and say in your findings that the data could not be checked.\n");
    }
    for src in &sources {
        let how = match src.kind {
            support_cases::SourceKind::Mongo => format!(
                "`tendril support db {name} --ticket {id} find <collection> --filter '<json>' [--projection '<json>'] [--sort '<json>'] [--limit N]`, also `count`, `aggregate --pipeline '<json array>'`, `collections`",
                name = src.name
            ),
            support_cases::SourceKind::Http => format!("`tendril support api {} --ticket {id} <path>` (GET only)", src.name),
        };
        let notes = if src.notes.is_empty() { String::new() } else { format!(" ({})", src.notes) };
        s.push_str(&format!("- **{}**{notes}: {how}\n", src.name));
    }
    s.push_str(&format!(
        r#"
## How to triage it
This ticket came in by itself and is yours, for project **{project}**. The partner sees nothing from here except the replies the rules below let through, and while approval is on, the operator approves each one first.

1. **Investigate, read-only.** Find out what is really happening: look up the account or the records being reported with the lookups above, and read the code involved. Small checks (a few lookups, a file or two) you do yourself; a larger investigation goes to a worker (`tendril manager task --project {project} ...`, deliverable a findings file). Change nothing: no code, no data, no deploys.
2. **Record what you found**, before any reply:
   `tendril support investigate {id} --category bug|question|request --severity critical|high|normal|low --confidence <0-100> --findings "<what is going on, in plain words>" --evidence "<file:line, collection and record, endpoint>" [--duplicate-of <ticket>]`
   Critical is only what Operations calls critical: a platform-wide outage (everyone's dashboards down, drawdown monitoring stopped for all accounts) or a breach. One account's problem, however urgent, is high at most. A critical ticket alerts the operator at once. Recording the investigation also sets the ticket's type on the Operations board.
3. **Then, by what it is:**
   - **Bug:** tell the partner you are on it: `tendril support reply {id} --kind ack "<short, human: we've found the problem and are fixing it>"`. Hand the fix to a worker, with a test that reproduces the bug. When the fix is a micro change (a few lines), its PR is merged or green, and you are 100% sure it fixes what the partner reported (not a guess): record `--confidence 100` and reply `tendril support reply {id} --kind resolved --fix-pr <url> --test "<the test that reproduces it and now passes>" --close "<it's fixed and goes out in the next update>"`. Anything bigger or less certain: an internal note for the operator, because they will step in.
   - **Question about the system:** research it. At 90% sure or more, answer short and simple, like a person: `tendril support reply {id} --kind answer "<answer>"`. Below 90%, an internal note with what you found.
   - **Request (a feature, a change, a favour):** do nothing on it and send no reply. Record it as a request, and the operator decides.
4. **Below the bar, or unsure: an internal note for the operator only**: `tendril support note {id} "<what they need to know>"`. That hands the ticket to them. One note, not a running commentary: your findings already show at the top of the ticket, so the note says only what the operator has to decide or do, in two or three plain sentences.
5. **When it is settled**, write how in one or two sentences so the next ticket like it starts from it: `tendril support learn {id} "<how it was fixed or settled>"`.

Replies read like a person on the support team: short, warm, plain, a sentence or two. Never ids, file names, code, error dumps or anything about another account. Notes read like a message to a colleague, not a report: no Markdown, no lists, no file paths (those go in --evidence). While you work it the ticket shows as in progress, so don't post notes to say you've started. The reply command refuses one that breaks these rules and says why. Past tickets: `tendril support history "<words>"`.
"#
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_never_reaches_the_status() {
        let home = tempfile::tempdir().unwrap();
        assert!(!status(home.path()).configured);
        save_connection(
            home.path(),
            &SupportConnection {
                base_url: "https://ops.example".into(),
                email: "me@example.com".into(),
                password: "secret".into(),
                token: Some("tok".into()),
                user_name: Some("Me".into()),
            },
        )
        .unwrap();
        let s = serde_json::to_string(&status(home.path())).unwrap();
        assert!(s.contains("me@example.com") && !s.contains("secret") && !s.contains("tok"), "{s}");
    }

    #[test]
    fn a_handoff_is_recorded_once_and_closed_by_resolve() {
        let home = tempfile::tempdir().unwrap();
        record_handoff(home.path(), "t1", "A", "s1").unwrap();
        record_handoff(home.path(), "t1", "B", "s2").unwrap();
        let all = load_handoffs(home.path());
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].project.as_str(), all[0].session_id.as_str()), ("B", "s2"));
        mark_handoff_done(home.path(), "t1").unwrap();
        assert!(load_handoffs(home.path())[0].done_at.is_some());
    }

    #[test]
    fn the_handoff_prompt_keeps_the_partner_out_until_done() {
        let ticket = json!({
            "id": "abc", "subject": "Login broken", "status": "OPEN", "priority": "HIGH", "platform": "WEB",
            "partner": { "name": "Acme" },
            "messages": [
                { "authorName": "Jo", "content": "I can't log in", "isInternal": false, "createdAt": "2026-10-28T09:00:00Z" },
                { "authorName": "Ryan", "content": "Looks like SSO", "isInternal": true, "createdAt": "2026-10-28T09:05:00Z" }
            ]
        });
        let p = handoff_prompt(&ticket, "trader-passport", "check the IdP");
        assert!(p.contains("**Login broken**") && p.contains("Partner: Acme"), "{p}");
        assert!(p.contains("Ryan (internal note): Looks like SSO"), "{p}");
        assert!(p.contains("check the IdP"));
        assert!(p.contains("tendril support get abc"));
        assert!(p.contains("tendril support close abc --kind resolved"));
        assert!(p.contains("must not hear from you unless the operator says so"));
        assert!(p.contains("Change nothing until the operator says so"));
    }
}
