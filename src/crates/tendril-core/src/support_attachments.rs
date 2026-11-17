//! The files and images partners attach to support tickets.
//!
//! Operations keeps an attachment as a link on its message: a Slack file behind Operations' own
//! `/api/team/slack-file` proxy (a path on Operations, signed in as the operator), or a Discord or
//! portal-upload URL elsewhere. The daemon fetches each one once, with the operator's Operations
//! session where the link is Operations', and keeps it under
//! `<TendrilHome>/support-attachments/<ticket>/`, so:
//!
//! * the manager can read them (`tendril support attachments <ticket>` saves them and prints where;
//!   an image is looked at by opening the file), and
//! * the app can show them (`GET /api/support/tickets/:id/attachments/:message/:index`), since it
//!   cannot sign in to Operations itself.

use crate::error::{Result, TendrilError};
use crate::support::{load_connection, login, save_connection};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Larger than this is not downloaded: a partner's screen recording is not worth filling the disk for.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);
const SESSION_COOKIE: &str = "cc_session";

/// One attachment on one message, and where it is (or will be) kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub message_id: String,
    /// Its place among the message's attachments.
    pub index: usize,
    pub filename: String,
    /// As Operations gave it, when it did; otherwise guessed from the name.
    pub content_type: String,
    pub image: bool,
    pub author: String,
    pub url: String,
    pub path: PathBuf,
}

pub fn dir(home: &Path, ticket: &str) -> PathBuf {
    home.join("support-attachments").join(safe(ticket))
}

/// A name safe to put on disk: letters, digits, `.`, `-` and `_`, never a path.
fn safe(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    let cleaned: String = cleaned.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect();
    if cleaned.is_empty() { "file".into() } else { cleaned }
}

/// The type a file's name says it is, for an attachment that came without one.
pub fn guess_type(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Every attachment on a ticket (as `support::get_ticket` returns it), oldest message first.
pub fn list(home: &Path, ticket: &Value) -> Vec<Attachment> {
    let id = ticket["id"].as_str().unwrap_or_default();
    let mut out = Vec::new();
    for m in ticket["messages"].as_array().into_iter().flatten() {
        let message_id = m["id"].as_str().unwrap_or_default().to_string();
        for (index, a) in m["attachments"].as_array().into_iter().flatten().enumerate() {
            let url = a["url"].as_str().unwrap_or_default().trim().to_string();
            if url.is_empty() {
                continue;
            }
            let filename = a["filename"].as_str().filter(|f| !f.trim().is_empty()).unwrap_or("attachment").trim().to_string();
            let content_type = a["type"]
                .as_str()
                .or_else(|| a["content_type"].as_str())
                .filter(|t| t.contains('/'))
                .map(str::to_string)
                .unwrap_or_else(|| guess_type(&filename).to_string());
            let image = content_type.starts_with("image/") || guess_type(&filename).starts_with("image/");
            let path = dir(home, id).join(format!("{}-{index}-{}", safe(&message_id), safe(&filename)));
            out.push(Attachment {
                message_id: message_id.clone(),
                index,
                filename,
                content_type,
                image,
                author: m["authorName"].as_str().unwrap_or_default().to_string(),
                url,
                path,
            });
        }
    }
    out
}

/// Fetches one attachment's bytes. A link on Operations goes with the operator's session (signing in
/// again once if it lapsed); a link anywhere else is fetched as it is, and never sent the session.
pub async fn fetch(home: &Path, url: &str) -> Result<(Vec<u8>, Option<String>)> {
    let client = reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default();
    let mut conn = load_connection(home);
    let base = conn.as_ref().map(|c| c.base_url.trim_end_matches('/').to_string()).unwrap_or_default();
    let (full, ours) = if url.starts_with('/') {
        if base.is_empty() {
            return Err(TendrilError::Validation("Support is not connected, so an Operations file cannot be fetched".into()));
        }
        (format!("{base}{url}"), true)
    } else {
        let ours = !base.is_empty() && (url == base || url.starts_with(&format!("{base}/")));
        (url.to_string(), ours)
    };
    for attempt in 0..2 {
        let mut req = client.get(&full);
        if ours {
            if let Some(c) = conn.as_mut() {
                if c.token.is_none() {
                    login(c).await?;
                    save_connection(home, c)?;
                }
                req = req.header("Cookie", format!("{SESSION_COOKIE}={}", c.token.as_deref().unwrap_or_default()));
            }
        }
        let resp = req.send().await.map_err(|e| TendrilError::Other(format!("Could not fetch the attachment: {}", e.without_url())))?;
        let status = resp.status();
        if ours && status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            if let Some(c) = conn.as_mut() {
                login(c).await?;
                save_connection(home, c)?;
                continue;
            }
        }
        if !status.is_success() {
            return Err(TendrilError::Other(format!("The attachment could not be fetched ({status}); the link may have expired")));
        }
        if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
            return Err(TendrilError::Other(format!("The attachment is larger than {} MB", MAX_BYTES / 1024 / 1024)));
        }
        let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = resp.bytes().await.map_err(|e| TendrilError::Other(format!("The attachment did not download: {}", e.without_url())))?;
        if bytes.len() > MAX_BYTES {
            return Err(TendrilError::Other(format!("The attachment is larger than {} MB", MAX_BYTES / 1024 / 1024)));
        }
        return Ok((bytes.to_vec(), content_type));
    }
    Err(TendrilError::Other("Operations kept refusing the session".into()))
}

/// Makes sure one attachment is on disk, fetching it only the first time.
pub async fn ensure(home: &Path, a: &Attachment) -> Result<()> {
    if a.path.is_file() {
        return Ok(());
    }
    let (bytes, _) = fetch(home, &a.url).await?;
    if let Some(parent) = a.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::fs_lock::write_atomic(&a.path, &bytes)?;
    Ok(())
}

/// Saves every attachment on a ticket that is not saved yet. Each one that could not be is returned
/// with why, so one expired link does not hide the rest.
pub async fn ensure_all(home: &Path, ticket: &Value) -> Vec<(Attachment, Option<String>)> {
    let mut out = Vec::new();
    for a in list(home, ticket) {
        let problem = ensure(home, &a).await.err().map(|e| e.to_string());
        out.push((a, problem));
    }
    out
}

/// One attachment's bytes, by the message it is on and its place there.
pub async fn read(home: &Path, ticket: &Value, message_id: &str, index: usize) -> Result<(Attachment, Vec<u8>)> {
    let a = list(home, ticket)
        .into_iter()
        .find(|a| a.message_id == message_id && a.index == index)
        .ok_or_else(|| TendrilError::Validation("That attachment is not on this ticket".into()))?;
    ensure(home, &a).await?;
    let bytes = std::fs::read(&a.path)?;
    Ok((a, bytes))
}

/// How a message's attachments read in a manager's prompt: one line each, under the message.
pub fn prompt_lines(message: &Value) -> String {
    let mut s = String::new();
    for a in message["attachments"].as_array().into_iter().flatten() {
        let name = a["filename"].as_str().unwrap_or("attachment");
        let kind = if a["type"].as_str().is_some_and(|t| t.starts_with("image/")) || guess_type(name).starts_with("image/") { "image" } else { "file" };
        s.push_str(&format!("    - attached {kind}: {name}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ticket() -> Value {
        json!({
            "id": "cmt1",
            "messages": [
                { "id": "m1", "authorName": "Jo", "content": "see screenshot", "attachments": [
                    { "url": "/api/team/slack-file?url=https%3A%2F%2Ffiles.slack.com%2Fx.png", "filename": "Screen Shot 1.png" },
                    { "url": "https://cdn.discordapp.com/a/b/log.txt", "filename": "log.txt", "type": "text/plain" },
                    { "url": "", "filename": "broken" }
                ]},
                { "id": "m2", "authorName": "Ryan", "content": "thanks" }
            ]
        })
    }

    #[test]
    fn attachments_are_listed_with_a_safe_place_on_disk() {
        let home = Path::new("/h");
        let all = list(home, &ticket());
        assert_eq!(all.len(), 2, "an empty link is skipped");
        assert!(all[0].image && all[0].content_type == "image/png" && all[0].author == "Jo");
        assert_eq!(all[0].path, Path::new("/h/support-attachments/cmt1/m1-0-Screen_Shot_1.png"));
        assert!(!all[1].image && all[1].content_type == "text/plain" && all[1].index == 1);
    }

    #[test]
    fn a_name_never_becomes_a_path() {
        assert_eq!(safe("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe(""), "file");
        assert_eq!(safe("ok-name_1.PNG"), "ok-name_1.PNG");
    }

    #[test]
    fn the_prompt_names_each_attachment_and_its_kind() {
        let t = ticket();
        let lines = prompt_lines(&t["messages"][0]);
        assert!(lines.contains("attached image: Screen Shot 1.png") && lines.contains("attached file: log.txt"), "{lines}");
        assert!(prompt_lines(&t["messages"][1]).is_empty());
    }
}
