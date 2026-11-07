//! What a manager did on the turns the daemon started: the patrol, its own wake-ups, worker reports,
//! mission changes. Those turns stay out of the chat and Telegram (see
//! [`tendril_core::chat::manager_brief::QUIET_REPLY`]), so this log is how the operator can still see
//! them when they want to, without being told each time.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tendril_core::chat::manager_brief::quiet_summary;
use tendril_core::chat::ChatSession;

/// Entries kept per project; older ones are dropped when the file is rewritten.
const KEEP: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub at: chrono::DateTime<chrono::Utc>,
    /// What woke it, in a few words.
    pub trigger: String,
    /// What it did.
    pub summary: String,
    /// It stayed quiet; otherwise the reply also went to the chat.
    pub quiet: bool,
}

fn log_path(home: &Path, project: &str) -> PathBuf {
    let safe: String = project.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    home.join("manager-activity").join(format!("{safe}.jsonl"))
}

fn clip(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    format!("{}…", one_line.chars().take(max - 1).collect::<String>())
}

/// What woke the manager, from the daemon's message: its first sentence.
fn trigger_of(prompt: &str) -> String {
    let first = prompt.trim().lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let first = first.trim_start_matches(['#', '*', ' ']);
    clip(first.split(". ").next().unwrap_or(first), 120)
}

/// The activity for the turn that just ended, when the daemon started it.
pub fn from_turn(session: &ChatSession) -> Option<Activity> {
    let idx = session.messages.iter().rposition(|m| m.role == "assistant" && !m.content.trim().is_empty())?;
    let prompt = session.messages[..idx].iter().rev().find(|m| m.role != "assistant")?;
    if prompt.role != "system" {
        return None;
    }
    let reply = &session.messages[idx];
    let (summary, quiet) = match quiet_summary(&reply.content) {
        Some(line) if line.is_empty() => ("Checked; nothing to do.".to_string(), true),
        Some(line) => (clip(&line, 240), true),
        None => (clip(&crate::push::plain(&reply.content), 240), false),
    };
    Some(Activity { at: reply.timestamp, trigger: trigger_of(&prompt.content), summary, quiet })
}

pub fn record(home: &Path, project: &str, activity: &Activity) {
    let path = log_path(home, project);
    let mut entries = load(home, project, KEEP);
    if entries.last() == Some(activity) {
        return;
    }
    entries.push(activity.clone());
    let start = entries.len().saturating_sub(KEEP);
    let body: String = entries[start..]
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|line| line + "\n")
        .collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, body);
}

/// The most recent `limit` entries, oldest first.
pub fn load(home: &Path, project: &str, limit: usize) -> Vec<Activity> {
    let entries: Vec<Activity> = std::fs::read_to_string(log_path(home, project))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let start = entries.len().saturating_sub(limit);
    entries[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trigger_is_the_first_sentence_of_the_daemon_message() {
        assert_eq!(trigger_of("Mission m1 \"BlackArrow\" is paused. Reason: budget."), "Mission m1 \"BlackArrow\" is paused");
        assert_eq!(trigger_of("\n## Patrol\n- stuck"), "Patrol");
    }

    #[test]
    fn the_log_keeps_the_latest_and_skips_a_repeat() {
        let home = std::env::temp_dir().join(format!("tendril-activity-{}", uuid::Uuid::new_v4()));
        let a = Activity { at: chrono::Utc::now(), trigger: "Patrol".into(), summary: "Merged PR 12.".into(), quiet: true };
        record(&home, "Acme", &a);
        record(&home, "Acme", &a);
        assert_eq!(load(&home, "Acme", 10), vec![a]);
        let _ = std::fs::remove_dir_all(home);
    }
}
