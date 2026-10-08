//! Watching a pull request's checks on a manager's behalf.
//!
//! The manager's worst miss was calling a PR done while the check that had originally failed was still
//! running. `tendril manager watch-pr --project P --pr 46` registers the PR here; the daemon asks GitHub
//! (through `gh`, already signed in on this machine) every minute, and wakes the manager once every check
//! has finished, with the result. A PR that merged before its checks finished is still watched until they
//! do, because that is exactly when the result matters.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Mutex;

pub const POLL_EVERY_SECONDS: i64 = 60;
/// Checks still running after this long are reported as stuck rather than watched forever.
pub const GIVE_UP_AFTER_SECONDS: i64 = 6 * 3600;
/// A PR whose checks list is still empty after this long has no CI to wait for.
pub const NO_CHECKS_AFTER_SECONDS: i64 = 10 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrWatch {
    pub id: String,
    pub project: String,
    pub pr: u64,
    /// `owner/name`, when the PR is not in the project's own repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub created_at: String,
    pub next_check_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WatchFile {
    watches: Vec<PrWatch>,
}

static FILE_LOCK: Mutex<()> = Mutex::const_new(());

fn path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("manager-pr-watches.json")
}

fn read(tendril_home: &Path) -> WatchFile {
    std::fs::read_to_string(path(tendril_home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write(tendril_home: &Path, file: &WatchFile) {
    let _ = std::fs::write(path(tendril_home), serde_json::to_vec_pretty(file).unwrap_or_default());
}

pub async fn add_watch(tendril_home: &Path, project: &str, pr: u64, repo: Option<String>) -> PrWatch {
    let now = chrono::Utc::now().to_rfc3339();
    let watch = PrWatch {
        id: uuid::Uuid::new_v4().to_string(),
        project: project.to_string(),
        pr,
        repo: repo.filter(|r| !r.trim().is_empty()),
        created_at: now.clone(),
        next_check_at: now,
    };
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(tendril_home);
    // Watching the same PR twice is one watch.
    file.watches.retain(|w| !(w.project == watch.project && w.pr == watch.pr && w.repo == watch.repo));
    file.watches.push(watch.clone());
    write(tendril_home, &file);
    watch
}

/// The watches due for a poll now, each pushed a minute into the future so a slow `gh` is not re-asked.
pub async fn due_watches(tendril_home: &Path) -> Vec<PrWatch> {
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(tendril_home);
    let now = chrono::Utc::now();
    let mut due = Vec::new();
    for w in &mut file.watches {
        let is_due = chrono::DateTime::parse_from_rfc3339(&w.next_check_at)
            .map(|d| d.with_timezone(&chrono::Utc) <= now)
            .unwrap_or(true);
        if is_due {
            due.push(w.clone());
            w.next_check_at = (now + chrono::Duration::seconds(POLL_EVERY_SECONDS)).to_rfc3339();
        }
    }
    if !due.is_empty() {
        write(tendril_home, &file);
    }
    due
}

pub async fn remove_watch(tendril_home: &Path, id: &str) {
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(tendril_home);
    file.watches.retain(|w| w.id != id);
    write(tendril_home, &file);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Some checks are still running or queued.
    Pending(usize),
    /// Every check finished and none failed.
    Green,
    /// Every check finished and these failed.
    Failed(Vec<String>),
    /// The PR has no checks at all.
    NoChecks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub title: String,
    /// `OPEN`, `MERGED` or `CLOSED`.
    pub state: String,
    pub outcome: CheckOutcome,
    /// How the branch stands against main, for an open pull request.
    pub against_main: AgainstMain,
}

/// Whether an open pull request's branch can go into main as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgainstMain {
    /// Up to date, or GitHub has not said otherwise.
    #[default]
    Fine,
    /// Main has moved on and the branch does not have it.
    Behind,
    /// It conflicts with main.
    Conflicts,
}

/// Reads `gh pr view --json title,state,statusCheckRollup`.
pub fn evaluate(json: &Value) -> PrStatus {
    let title = json["title"].as_str().unwrap_or("").to_string();
    let state = json["state"].as_str().unwrap_or("OPEN").to_string();
    let against_main = if state != "OPEN" {
        AgainstMain::Fine
    } else if json["mergeable"].as_str() == Some("CONFLICTING") || json["mergeStateStatus"].as_str() == Some("DIRTY") {
        AgainstMain::Conflicts
    } else if json["mergeStateStatus"].as_str() == Some("BEHIND") {
        AgainstMain::Behind
    } else {
        AgainstMain::Fine
    };
    let checks = json["statusCheckRollup"].as_array().cloned().unwrap_or_default();
    if checks.is_empty() {
        return PrStatus { title, state, outcome: CheckOutcome::NoChecks, against_main };
    }
    let mut pending = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for check in &checks {
        let name = check["name"].as_str().or_else(|| check["context"].as_str()).unwrap_or("a check").to_string();
        if let Some(status) = check["status"].as_str() {
            // A check run.
            if !status.eq_ignore_ascii_case("COMPLETED") {
                pending += 1;
                continue;
            }
            match check["conclusion"].as_str().unwrap_or("").to_ascii_uppercase().as_str() {
                "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => failed.push(name),
                _ => {}
            }
        } else {
            // A commit status.
            match check["state"].as_str().unwrap_or("").to_ascii_uppercase().as_str() {
                "PENDING" | "EXPECTED" => pending += 1,
                "FAILURE" | "ERROR" => failed.push(name),
                _ => {}
            }
        }
    }
    let outcome = if pending > 0 {
        CheckOutcome::Pending(pending)
    } else if failed.is_empty() {
        CheckOutcome::Green
    } else {
        failed.sort();
        failed.dedup();
        CheckOutcome::Failed(failed)
    };
    PrStatus { title, state, outcome, against_main }
}

/// Asks GitHub about the PR from inside the project's repo.
pub async fn fetch(watch: &PrWatch, repo_dir: &Path) -> Result<PrStatus, String> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.current_dir(repo_dir)
        .args(["pr", "view", &watch.pr.to_string(), "--json", "title,state,statusCheckRollup,mergeable,mergeStateStatus"]);
    if let Some(repo) = &watch.repo {
        cmd.args(["--repo", repo]);
    }
    let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| "gh timed out".to_string())?
        .map_err(|e| format!("could not run gh: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let json: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    Ok(evaluate(&json))
}

/// The sentence the manager is woken with, or `None` while the checks are still running.
pub fn wake_message(watch: &PrWatch, status: &PrStatus, age_seconds: i64) -> Option<String> {
    let subject = format!("PR {} \"{}\" ({})", watch.pr, status.title, status.state.to_lowercase());
    // Green against an old main is not green: the branch has to take main in and be checked again.
    let stale = match status.against_main {
        AgainstMain::Fine => None,
        AgainstMain::Behind => Some(format!(
            "{subject}: main has moved on and this branch does not have it, so it is not ready. Bring main in (`gh pr update-branch {}`), then watch it again.",
            watch.pr
        )),
        AgainstMain::Conflicts => Some(format!(
            "{subject}: it conflicts with main, so it is not ready. Hand out a task on the PR branch to merge the latest main in and resolve the conflicts, have it pushed, then watch it again."
        )),
    };
    match &status.outcome {
        CheckOutcome::Pending(_) if stale.is_some() && status.against_main == AgainstMain::Conflicts => stale,
        CheckOutcome::Green | CheckOutcome::NoChecks if stale.is_some() => stale,
        CheckOutcome::Green => Some(format!("{subject}: every check passed, and GitHub reports it neither behind nor in conflict with main.")),
        CheckOutcome::Failed(names) => Some(format!(
            "{subject}: checks failed: {}. Look at the failing runs and delegate a fix on the PR branch.",
            names.join(", ")
        )),
        CheckOutcome::NoChecks if age_seconds >= NO_CHECKS_AFTER_SECONDS => {
            Some(format!("{subject}: it has no checks to wait for."))
        }
        CheckOutcome::Pending(n) if age_seconds >= GIVE_UP_AFTER_SECONDS => Some(format!(
            "{subject}: {n} check(s) are still running after {} hours. They may be stuck.",
            GIVE_UP_AFTER_SECONDS / 3600
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn watch() -> PrWatch {
        PrWatch {
            id: "w".into(),
            project: "Acme".into(),
            pr: 46,
            repo: None,
            created_at: String::new(),
            next_check_at: String::new(),
        }
    }

    #[test]
    fn running_checks_keep_it_pending_even_when_the_pr_already_merged() {
        let s = evaluate(&json!({
            "title": "Module abstraction", "state": "MERGED",
            "statusCheckRollup": [
                {"name": "Build & test", "status": "IN_PROGRESS", "conclusion": ""},
                {"name": "Lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
            ]
        }));
        assert_eq!(s.outcome, CheckOutcome::Pending(1));
        assert_eq!(wake_message(&watch(), &s, 60), None, "no news while a check is still running");
    }

    #[test]
    fn a_failure_is_named_once_all_checks_have_finished() {
        let s = evaluate(&json!({
            "title": "t", "state": "OPEN",
            "statusCheckRollup": [
                {"name": "Build & test", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"name": "Build & test", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"name": "Lint", "status": "COMPLETED", "conclusion": "SKIPPED"},
                {"context": "ci/legacy", "state": "ERROR"},
            ]
        }));
        assert_eq!(s.outcome, CheckOutcome::Failed(vec!["Build & test".into(), "ci/legacy".into()]));
        assert!(wake_message(&watch(), &s, 60).unwrap().contains("checks failed: Build & test, ci/legacy"));
    }

    #[test]
    fn skipped_and_neutral_count_as_passing() {
        let s = evaluate(&json!({
            "title": "t", "state": "OPEN",
            "statusCheckRollup": [
                {"name": "a", "status": "COMPLETED", "conclusion": "SKIPPED"},
                {"name": "b", "status": "COMPLETED", "conclusion": "NEUTRAL"},
                {"name": "c", "status": "COMPLETED", "conclusion": "SUCCESS"},
            ]
        }));
        assert_eq!(s.outcome, CheckOutcome::Green);
        assert!(wake_message(&watch(), &s, 5).unwrap().contains("every check passed"));
    }

    #[test]
    fn no_checks_only_means_done_after_a_while_and_stuck_runs_are_reported() {
        let none = evaluate(&json!({"title": "t", "state": "OPEN", "statusCheckRollup": []}));
        assert_eq!(wake_message(&watch(), &none, 30), None, "CI may simply not have started yet");
        assert!(wake_message(&watch(), &none, NO_CHECKS_AFTER_SECONDS).is_some());
        let stuck = PrStatus { title: "t".into(), state: "OPEN".into(), outcome: CheckOutcome::Pending(2), against_main: AgainstMain::Fine };
        assert!(wake_message(&watch(), &stuck, GIVE_UP_AFTER_SECONDS).unwrap().contains("still running"));
    }

    #[test]
    fn green_checks_on_a_branch_that_lacks_main_are_not_ready() {
        let checks = json!([{"name": "c", "status": "COMPLETED", "conclusion": "SUCCESS"}]);
        let behind = evaluate(&json!({"title": "t", "state": "OPEN", "mergeStateStatus": "BEHIND", "statusCheckRollup": checks}));
        assert_eq!(behind.against_main, AgainstMain::Behind);
        assert!(wake_message(&watch(), &behind, 60).unwrap().contains("gh pr update-branch 46"));
        let conflicts = evaluate(&json!({"title": "t", "state": "OPEN", "mergeable": "CONFLICTING", "statusCheckRollup": [{"name": "c", "status": "IN_PROGRESS"}]}));
        // A conflict is news straight away: no point waiting for checks that will have to run again.
        assert!(wake_message(&watch(), &conflicts, 60).unwrap().contains("conflicts with main"));
        // Once merged, how it stood against main no longer matters.
        let merged = evaluate(&json!({"title": "t", "state": "MERGED", "mergeStateStatus": "BEHIND", "statusCheckRollup": checks}));
        assert!(wake_message(&watch(), &merged, 60).unwrap().contains("every check passed"));
    }

    #[tokio::test]
    async fn watching_the_same_pr_twice_is_one_watch_and_due_ones_are_pushed_back() {
        let home = std::env::temp_dir().join(format!("tendril-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        add_watch(&home, "Acme", 46, None).await;
        add_watch(&home, "Acme", 46, None).await;
        assert_eq!(read(&home).watches.len(), 1);
        assert_eq!(due_watches(&home).await.len(), 1, "a new watch is due straight away");
        assert!(due_watches(&home).await.is_empty(), "and then not again for a minute");
        let id = read(&home).watches[0].id.clone();
        remove_watch(&home, &id).await;
        assert!(read(&home).watches.is_empty());
        let _ = std::fs::remove_dir_all(home);
    }
}
