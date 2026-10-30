//! Two Claude subscriptions behind the one `claude` agent: the main account, and a fallback that takes
//! over while the main one is rate limited.
//!
//! Each account is its own Claude Code config directory, so each keeps its own login. Signing the
//! fallback in never touches the main account's credentials, which is why going back to main needs
//! no new sign-in (logging a second account into the same directory replaces the first one's token).
//! The main account is wherever Claude Code normally keeps its config. The fallback is
//! `~/.claude-fallback`: everything except the login is linked to the main directory, so sessions,
//! settings, skills and plugins are shared. That includes the transcripts a resumed manager session
//! is read back from, so a session can carry on from one account to the other.
//!
//! Every run of the `claude` binary goes through [`account_for_run`] (from `agents::runner`). Main is
//! used unless it is limited and the fallback is signed in and is not limited. A limit is recorded
//! against the account that hit it, and the agent as a whole counts as limited only once both are, so
//! the agent-level fallback chain (Codex, say) only takes over after both subscriptions run out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::dirs_home;

pub const MAIN: &str = "main";
pub const FALLBACK: &str = "fallback";
/// The login-session id the sign-in dialog uses for the fallback account.
pub const FALLBACK_LOGIN_ID: &str = "claude-fallback";

/// Never linked into the fallback directory: these hold the login and the account it belongs to.
const OWN_FILES: &[&str] = &[".credentials.json", ".claude.json", ".claude.json.backup"];

static LIMITS: Mutex<Option<HashMap<&'static str, DateTime<Utc>>>> = Mutex::new(None);

fn with_limits<T>(f: impl FnOnce(&mut HashMap<&'static str, DateTime<Utc>>) -> T) -> T {
    let mut guard = LIMITS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Claude Code's own config directory: `CLAUDE_CONFIG_DIR` when the daemon has one, else `~/.claude`.
pub fn main_dir() -> Option<PathBuf> {
    match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.trim().is_empty() => Some(PathBuf::from(dir.trim())),
        _ => Some(dirs_home()?.join(".claude")),
    }
}

pub fn fallback_dir() -> Option<PathBuf> {
    Some(dirs_home()?.join(".claude-fallback"))
}

/// Where an account's `.claude.json` (which names the signed-in account) lives. With the default
/// directory Claude Code keeps it in the home directory rather than inside `~/.claude`.
fn account_file(account: &str) -> Option<PathBuf> {
    if account == FALLBACK {
        return Some(fallback_dir()?.join(".claude.json"));
    }
    match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.trim().is_empty() => Some(Path::new(dir.trim()).join(".claude.json")),
        _ => Some(dirs_home()?.join(".claude.json")),
    }
}

/// The email of the account signed in to `account`, if one is.
pub fn signed_in_email(account: &str) -> Option<String> {
    let text = std::fs::read_to_string(account_file(account)?).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let oauth = json.get("oauthAccount")?;
    Some(
        oauth
            .get("emailAddress")
            .and_then(|v| v.as_str())
            .unwrap_or("signed in")
            .to_string(),
    )
}

/// Whether a fallback account is signed in.
pub fn has_fallback() -> bool {
    signed_in_email(FALLBACK).is_some()
}

/// Makes the fallback directory, linking everything in the main one except the login into it.
/// Safe to call on every run: only missing links are created.
pub fn prepare_fallback_dir() -> std::io::Result<PathBuf> {
    let dir = fallback_dir().ok_or_else(|| std::io::Error::other("No home directory"))?;
    std::fs::create_dir_all(&dir)?;
    let Some(main) = main_dir() else { return Ok(dir) };
    // The two shared directories a resumed or configured session needs, made first so they are
    // links even before the main account has ever written them.
    for shared in ["projects", "todos"] {
        let _ = std::fs::create_dir_all(main.join(shared));
    }
    if let Ok(entries) = std::fs::read_dir(&main) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else { continue };
            if OWN_FILES.contains(&name_str) {
                continue;
            }
            let link = dir.join(&name);
            if link.symlink_metadata().is_ok() {
                continue;
            }
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(entry.path(), &link);
        }
    }
    Ok(dir)
}

/// When `account` is limited until, if it is at `now`.
pub fn limited_until(account: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    with_limits(|m| m.get(account).copied().filter(|until| *until > now))
}

/// Which account the next run uses: main, unless it is limited and a signed-in fallback is not.
pub fn pick(now: DateTime<Utc>) -> &'static str {
    if limited_until(MAIN, now).is_some() && has_fallback() && limited_until(FALLBACK, now).is_none() {
        FALLBACK
    } else {
        MAIN
    }
}

/// The `CLAUDE_CONFIG_DIR` a `claude` run should get and the account it is, or `None` to leave the
/// run alone (it is already pointed somewhere, or it runs on main).
pub fn account_for_run(command: &str, environment: &HashMap<String, String>) -> Option<(&'static str, Option<PathBuf>)> {
    let binary = Path::new(command).file_stem()?.to_str()?;
    if binary != "claude" || environment.contains_key("CLAUDE_CONFIG_DIR") {
        return None;
    }
    match pick(Utc::now()) {
        FALLBACK => prepare_fallback_dir().ok().map(|dir| (FALLBACK, Some(dir))),
        _ => Some((MAIN, None)),
    }
}

static LAST_SWITCH: Mutex<Option<DateTime<Utc>>> = Mutex::new(None);

/// Whether an account hit its limit in the last two minutes and the other one has taken over: what
/// tells a caller that retrying the same work at once will run on the other account, not loop.
pub fn just_switched(now: DateTime<Utc>) -> bool {
    let last = *LAST_SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    last.is_some_and(|at| now - at <= chrono::Duration::minutes(2)) && has_fallback() && another_account_available(now)
}

/// Records that `account` hit its limit until `until` (the later of this and any earlier record).
pub fn note_limited(account: &'static str, until: DateTime<Utc>) {
    *LAST_SWITCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(Utc::now());
    with_limits(|m| {
        let entry = m.entry(account).or_insert(until);
        if until > *entry {
            *entry = until;
        }
    });
}

/// Whether another Claude account can take the next run at `now`. When one can, a limit on the
/// `claude` agent is the account's business, not the agent's.
pub fn another_account_available(now: DateTime<Utc>) -> bool {
    limited_until(MAIN, now).is_none() || (has_fallback() && limited_until(FALLBACK, now).is_none())
}

/// Signs the fallback out: its directory goes, links and all (the links only, never what they point at).
pub fn remove_fallback() -> std::io::Result<()> {
    with_limits(|m| {
        m.remove(FALLBACK);
    });
    match fallback_dir() {
        Some(dir) if dir.exists() => std::fs::remove_dir_all(dir),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub id: &'static str,
    pub email: Option<String>,
    pub limited_until: Option<DateTime<Utc>>,
    /// Whether the next run would use this account.
    pub active: bool,
}

pub fn status() -> Vec<AccountStatus> {
    let now = Utc::now();
    let active = pick(now);
    [MAIN, FALLBACK]
        .into_iter()
        .map(|id| AccountStatus {
            id,
            email: signed_in_email(id),
            limited_until: limited_until(id, now),
            active: id == active,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_claude_run_is_given_an_account() {
        let env = HashMap::new();
        assert!(account_for_run("codex", &env).is_none());
        assert!(account_for_run("/usr/local/bin/claude", &env).is_some());
        let pinned = HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())]);
        assert!(account_for_run("claude", &pinned).is_none());
    }
}
