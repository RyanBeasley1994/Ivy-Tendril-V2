//! The one Claude Code session a project's manager lives in.
//!
//! A chat turn is normally a fresh agent that is handed the conversation so far, so every turn costs
//! more than the one before and nothing is ever cached. A manager's session lasts as long as its
//! project and is woken by the daemon many times a day, which makes that the most expensive way to run
//! it. Instead its first turn opens a Claude Code session, and every turn after that resumes it with
//! only what is new: Claude Code keeps the conversation, caches it, and compacts it when it grows.
//!
//! This file remembers which session that is. Deleting the manager's chat (the app's `/clear`) forgets
//! it, so the next turn opens a new one.
//!
//! **A session does not live forever.** Claude Code only compacts near the model's own context limit,
//! and every tool call in a turn re-reads the whole context, so a manager woken all day ends up paying
//! for hundreds of thousands of tokens per call. Once a turn's context passes [`MAX_CONTEXT_TOKENS`]
//! the session is forgotten and the next turn opens a fresh one: briefing, the project's snapshot, the
//! Goals memory and the latest messages, which is everything a manager is told to rely on anyway.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    /// Claude Code's own session id.
    pub id: String,
    /// Where it was opened. Claude Code files a session under the directory it started in, so it can
    /// only be resumed from there.
    pub directory: String,
}

fn path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("manager-agent-sessions.json")
}

fn read(tendril_home: &Path) -> HashMap<String, AgentSession> {
    std::fs::read_to_string(path(tendril_home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write(tendril_home: &Path, all: &HashMap<String, AgentSession>) {
    let _ = std::fs::write(path(tendril_home), serde_json::to_vec_pretty(all).unwrap_or_default());
}

/// The agent session a chat session is living in, if it has one.
pub fn get(tendril_home: &Path, chat_session: &str) -> Option<AgentSession> {
    read(tendril_home).remove(chat_session)
}

pub fn set(tendril_home: &Path, chat_session: &str, session: AgentSession) {
    let mut all = read(tendril_home);
    if all.get(chat_session) != Some(&session) {
        all.insert(chat_session.to_string(), session);
        write(tendril_home, &all);
    }
}

/// Forgets a chat session's agent session, so its next turn opens a new one.
pub fn forget(tendril_home: &Path, chat_session: &str) {
    let mut all = read(tendril_home);
    if all.remove(chat_session).is_some() {
        write(tendril_home, &all);
    }
}

/// How large a manager's resumed context may grow before its next turn starts a fresh session.
pub const MAX_CONTEXT_TOKENS: u64 = 120_000;

/// The context one model request carried, from a Claude Code `assistant` stream line's usage: input,
/// cache reads and cache writes together are what the model read for that call.
pub fn context_tokens_in(raw_line: &str) -> Option<u64> {
    if !raw_line.contains("\"usage\"") || !raw_line.contains("\"assistant\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw_line.trim()).ok()?;
    if value["type"].as_str() != Some("assistant") {
        return None;
    }
    let usage = &value["message"]["usage"];
    let total = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
        .iter()
        .map(|k| usage[*k].as_u64().unwrap_or(0))
        .sum::<u64>();
    (total > 0).then_some(total)
}

/// The session id Claude Code reports on a line of its stream, when the line carries one. Read every
/// turn rather than assumed, because a resumed session is not guaranteed to keep the id it was resumed by.
pub fn session_id_in(raw_line: &str) -> Option<String> {
    if !raw_line.contains("\"session_id\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw_line.trim()).ok()?;
    if !matches!(value["type"].as_str(), Some("system") | Some("result")) {
        return None;
    }
    value["session_id"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_remembered_until_it_is_forgotten() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(get(home.path(), "manager-acme"), None);
        let s = AgentSession { id: "abc".into(), directory: "/home".into() };
        set(home.path(), "manager-acme", s.clone());
        set(home.path(), "manager-other", AgentSession { id: "xyz".into(), directory: "/home".into() });
        assert_eq!(get(home.path(), "manager-acme"), Some(s));
        forget(home.path(), "manager-acme");
        assert_eq!(get(home.path(), "manager-acme"), None);
        assert!(get(home.path(), "manager-other").is_some(), "another manager's is untouched");
    }

    #[test]
    fn the_session_id_is_read_off_claudes_own_events_only() {
        assert_eq!(session_id_in(r#"{"type":"system","subtype":"init","session_id":"s-1"}"#).as_deref(), Some("s-1"));
        assert_eq!(session_id_in(r#"{"type":"result","subtype":"success","session_id":"s-2"}"#).as_deref(), Some("s-2"));
        // Text an agent wrote that happens to mention one is not it.
        assert_eq!(session_id_in(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"\"session_id\": \"x\""}]}}"#), None);
        assert_eq!(session_id_in("not json \"session_id\""), None);
        assert_eq!(session_id_in(r#"{"type":"result"}"#), None);
    }

    #[test]
    fn the_context_of_a_request_is_read_from_its_usage() {
        let line = r#"{"type":"assistant","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":90000,"cache_creation_input_tokens":3000,"output_tokens":400}}}"#;
        assert_eq!(context_tokens_in(line), Some(93_012));
        assert_eq!(context_tokens_in(r#"{"type":"result","usage":{"input_tokens":5}}"#), None);
        assert_eq!(context_tokens_in(r#"{"type":"assistant","message":{"content":[]}}"#), None);
    }
}
