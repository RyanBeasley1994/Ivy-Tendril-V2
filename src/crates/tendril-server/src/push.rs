//! Phone and desktop push from the daemon, so a manager can reach you when no app is open.
//!
//! The managers run on this machine all day; the only things worth interrupting you for are a manager
//! that needs you and a manager whose work is complete. Pushes go to an [ntfy](https://ntfy.sh) topic
//! (the public server or your own): a plain HTTP POST that the ntfy phone and desktop apps show as a
//! notification. Configure it with `tendril push set <topic-url>`, which writes `push.json` in the
//! Tendril home; `TENDRIL_PUSH_URL` / `TENDRIL_PUSH_TOKEN` in the environment work too.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BriefingConfig {
    /// Local wall-clock time to send it, `HH:MM`.
    pub time: String,
    /// Minutes east of UTC for that wall clock: the daemon's machine is usually on UTC.
    #[serde(default)]
    pub utc_offset_minutes: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PushConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub briefing: Option<BriefingConfig>,
    /// Where `push.json` was read from, so browser subscriptions (`webpush.json`) beside it are reached too.
    #[serde(skip)]
    pub home: Option<std::path::PathBuf>,
}

impl PushConfig {
    pub fn path(tendril_home: &Path) -> std::path::PathBuf {
        tendril_home.join("push.json")
    }

    pub fn load(tendril_home: &Path) -> Self {
        let mut cfg: PushConfig = std::fs::read_to_string(Self::path(tendril_home))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        cfg.home = Some(tendril_home.to_path_buf());
        if cfg.url.trim().is_empty() {
            if let Ok(url) = std::env::var("TENDRIL_PUSH_URL") {
                cfg.url = url;
            }
        }
        if cfg.token.is_none() {
            cfg.token = std::env::var("TENDRIL_PUSH_TOKEN").ok().filter(|t| !t.trim().is_empty());
        }
        cfg
    }

    /// An ntfy topic is set, or at least one browser has subscribed.
    pub fn enabled(&self) -> bool {
        !self.url.trim().is_empty() || self.web_subscribers() > 0
    }

    fn web_subscribers(&self) -> usize {
        self.home.as_deref().map(crate::webpush::subscription_count).unwrap_or(0)
    }
}

/// ntfy takes the message as the body and the title as a header. A header cannot carry newlines.
pub async fn send(cfg: &PushConfig, title: &str, body: &str, high_priority: bool) -> Result<(), String> {
    if let Some(home) = cfg.home.as_deref() {
        // The project is the title's first segment, so a newer notice for it replaces an older one.
        let tag = title.split(" · ").next().unwrap_or(title);
        crate::webpush::send_all(
            home,
            &crate::webpush::Notice { title, body, urgent: high_priority, tag, url: "/" },
        )
        .await;
    }
    if cfg.url.trim().is_empty() {
        return Ok(());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .post(cfg.url.trim())
        .header("Title", title.replace(['\n', '\r'], " "))
        .header("Priority", if high_priority { "high" } else { "default" })
        .body(body.to_string());
    if let Some(token) = &cfg.token {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("push service answered {}", resp.status()))
    }
}

/// Markdown stripped to the plain sentence a notification can show.
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        out.push_str(line);
        out.push(' ');
    }
    out.chars()
        .filter(|c| !matches!(c, '`' | '*' | '_' | '#' | '>'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

pub fn last_paragraph(text: &str) -> &str {
    text.trim().rsplit("\n\n").next().unwrap_or("").trim()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The manager asked something, or a mission is still waiting on a person.
    NeedsYou,
    /// Nothing is left running for the project.
    WorkComplete,
}

/// What a manager's finished turn means for you. Workers still going and nothing asked means the
/// manager is waiting on them, not on you, so there is nothing to say.
pub fn classify_turn_end(reply: &str, waiting_missions: usize, still_working: bool) -> Option<TurnOutcome> {
    let asked = last_paragraph(reply).ends_with('?');
    if asked || waiting_missions > 0 {
        Some(TurnOutcome::NeedsYou)
    } else if !still_working {
        Some(TurnOutcome::WorkComplete)
    } else {
        None
    }
}

/// A turn the operator started by typing, and that ended within this long, is one they watched.
pub const WATCHED_TURN_SECONDS: i64 = 120;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_or_a_waiting_mission_needs_you_and_idle_means_complete() {
        assert_eq!(classify_turn_end("Merged it. Want me to push?", 0, true), Some(TurnOutcome::NeedsYou));
        assert_eq!(classify_turn_end("Paused on the budget.", 1, true), Some(TurnOutcome::NeedsYou));
        assert_eq!(classify_turn_end("All done.", 0, false), Some(TurnOutcome::WorkComplete));
        assert_eq!(classify_turn_end("Two of three finished.", 0, true), None);
    }

    #[test]
    fn only_the_last_paragraph_decides_whether_it_asked() {
        let reply = "Why this matters?\n\nIt is merged and green.";
        assert_eq!(classify_turn_end(reply, 0, false), Some(TurnOutcome::WorkComplete));
        let asked = "It is merged.\n\nShall I push it?";
        assert_eq!(classify_turn_end(asked, 0, true), Some(TurnOutcome::NeedsYou));
    }

    #[test]
    fn markdown_becomes_a_plain_clipped_sentence() {
        let md = "## Done\n\n**Both** merged.\n```\nlong log\n```\nNext: `adapter`.";
        let text = plain(md);
        assert_eq!(text, "Done Both merged. Next: adapter.");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        assert_eq!(clip("abc", 5), "abc");
    }

    #[test]
    fn push_is_off_without_a_url_and_reads_the_file_when_there_is_one() {
        let home = std::env::temp_dir().join(format!("tendril-push-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        assert!(!PushConfig { url: " ".into(), ..Default::default() }.enabled());
        std::fs::write(
            PushConfig::path(&home),
            r#"{"url":"https://ntfy.sh/mine","briefing":{"time":"08:00","utcOffsetMinutes":60}}"#,
        )
        .unwrap();
        let cfg = PushConfig::load(&home);
        assert!(cfg.enabled());
        assert_eq!(cfg.briefing.unwrap().utc_offset_minutes, 60);
        let _ = std::fs::remove_dir_all(home);
    }
}
