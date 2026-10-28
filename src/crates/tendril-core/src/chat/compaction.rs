//! Keeps a manager's stored conversation small.
//!
//! A manager's chat lives as long as its project and is woken all day, so it only ever grows: a busy
//! one passed 600 messages and 7 MB in a day and a half, and every open of the project page and every
//! session list paid for it. Almost all of that weight is `raw_stream`, the turn's whole agent output,
//! which is only there to draw the expandable tool-call view of recent turns.
//!
//! After each turn, [`compact`]:
//! * keeps `raw_stream` only on the latest [`KEEP_RAW_STREAMS`] messages; older ones keep their text;
//! * once there are more than [`MAX_MESSAGES`], moves the oldest to
//!   `<TendrilHome>/Chats/Archive/<id>.jsonl` (one message per line, appended), never the briefing.
//!   `load_all_sessions` reads only `Chats/*.json`, so the archive costs nothing at load.
//!
//! None of this changes what the manager is told: its prompt already replays only its latest messages
//! (`prompt::MANAGER_HISTORY_MESSAGES`), and what must last is in its Goals memory.

use crate::chat::manager_brief::{is_briefing, is_manager_session};
use crate::chat::storage::get_chats_dir;
use crate::chat::ChatSession;
use std::io::Write;
use std::path::Path;

pub const KEEP_RAW_STREAMS: usize = 12;
pub const MAX_MESSAGES: usize = 200;
/// Archived in one go once over the cap, so the archive is written now and then, not every turn.
const ARCHIVE_DOWN_TO: usize = 150;

/// Compacts `session` in place when it is a manager's. Returns how many messages were archived.
pub fn compact(tendril_home: &Path, session: &mut ChatSession) -> usize {
    if !is_manager_session(&session.id) {
        return 0;
    }
    let len = session.messages.len();
    for m in session.messages.iter_mut().take(len.saturating_sub(KEEP_RAW_STREAMS)) {
        m.raw_stream = None;
    }
    if len <= MAX_MESSAGES {
        return 0;
    }
    let briefing = session.messages.iter().position(|m| m.role == "system" && is_briefing(&m.content));
    let mut keep = Vec::with_capacity(ARCHIVE_DOWN_TO + 1);
    let mut archived = Vec::new();
    let cut = len - ARCHIVE_DOWN_TO;
    for (i, m) in std::mem::take(&mut session.messages).into_iter().enumerate() {
        if i < cut && Some(i) != briefing {
            archived.push(m);
        } else {
            keep.push(m);
        }
    }
    if let Err(e) = append_archive(tendril_home, &session.id, &archived) {
        // Never drop history that could not be written down: put it back and try next turn.
        tracing::warn!("could not archive {} messages of {}: {e}", archived.len(), session.id);
        archived.extend(keep);
        session.messages = archived;
        session.messages.sort_by_key(|m| m.timestamp);
        return 0;
    }
    session.messages = keep;
    archived.len()
}

fn append_archive(tendril_home: &Path, id: &str, messages: &[crate::chat::ChatMessage]) -> std::io::Result<()> {
    let dir = get_chats_dir(tendril_home).join("Archive");
    std::fs::create_dir_all(&dir)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{id}.jsonl")))?;
    for m in messages {
        let line = serde_json::to_string(m).map_err(std::io::Error::other)?;
        writeln!(file, "{line}")?;
    }
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatMessage;
    use chrono::{Duration, Utc};

    fn msg(i: usize, role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: format!("m{i}"),
            role: role.into(),
            content: content.into(),
            timestamp: Utc::now() + Duration::seconds(i as i64),
            agent_id: None,
            model_id: None,
            raw_stream: Some("raw".into()),
            effort: None,
        }
    }

    fn session(id: &str, n: usize) -> ChatSession {
        let mut s: ChatSession = serde_json::from_value(serde_json::json!({
            "id": id, "title": "t", "createdAt": Utc::now(), "updatedAt": Utc::now(), "agentId": "claude", "modelId": "", "messages": []
        }))
        .unwrap();
        s.messages.push(msg(0, "system", "# You are the Factory Manager for project \"P\""));
        for i in 1..n {
            s.messages.push(msg(i, if i % 2 == 0 { "user" } else { "assistant" }, &format!("message {i}")));
        }
        s
    }

    #[test]
    fn only_recent_messages_keep_their_raw_stream() {
        let home = tempfile::tempdir().unwrap();
        let mut s = session("manager-p", 40);
        assert_eq!(compact(home.path(), &mut s), 0);
        let with_raw = s.messages.iter().filter(|m| m.raw_stream.is_some()).count();
        assert_eq!(with_raw, KEEP_RAW_STREAMS);
        assert!(s.messages.last().unwrap().raw_stream.is_some());
    }

    #[test]
    fn past_the_cap_the_oldest_are_archived_and_the_briefing_stays() {
        let home = tempfile::tempdir().unwrap();
        let mut s = session("manager-p", MAX_MESSAGES + 10);
        let archived = compact(home.path(), &mut s);
        // Everything before the last ARCHIVE_DOWN_TO, except the briefing at the front.
        assert_eq!(archived, MAX_MESSAGES + 10 - ARCHIVE_DOWN_TO - 1);
        assert_eq!(s.messages.len(), ARCHIVE_DOWN_TO + 1);
        assert!(is_briefing(&s.messages[0].content));
        assert_eq!(s.messages.last().unwrap().content, format!("message {}", MAX_MESSAGES + 9));
        let lines = std::fs::read_to_string(home.path().join("Chats/Archive/manager-p.jsonl")).unwrap();
        assert_eq!(lines.lines().count(), archived);
    }

    #[test]
    fn other_chats_are_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let mut s = session("0d1f-a-chat", MAX_MESSAGES + 10);
        assert_eq!(compact(home.path(), &mut s), 0);
        assert!(s.messages.iter().all(|m| m.raw_stream.is_some()));
    }
}
