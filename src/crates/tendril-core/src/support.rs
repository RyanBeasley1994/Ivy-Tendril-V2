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
        }
    }
    Ok(value)
}

/// The sidebar badge: the operator's unread tickets in "Needs Us", as Operations counts them.
pub async fn unread_count(home: &Path) -> Result<u64> {
    let v = request(home, Method::GET, "/api/team/tickets/count", None).await?;
    Ok(v["count"].as_u64().unwrap_or(0))
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

/// Operations' silent resolve: no message to the partner, notifications cleared.
pub async fn resolve(home: &Path, id: &str) -> Result<Value> {
    let v = request(home, Method::POST, &format!("/api/team/tickets/{}/resolve", enc(id)), None).await?;
    mark_handoff_done(home, id)?;
    Ok(v)
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

When the operator says to reply and close (for example once an agreed fix is merged), do it in one step:
`tendril support close {id} --message "<what they agreed to tell the partner>"`
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
        assert!(p.contains("tendril support close abc --message"));
        assert!(p.contains("must not hear from you unless the operator says so"));
        assert!(p.contains("Change nothing until the operator says so"));
    }
}
