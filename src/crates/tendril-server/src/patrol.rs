//! Managers that manage without being asked.
//!
//! A manager only runs a turn when something prompts it, so left alone it sits idle until the operator
//! types. Two prompts fix that:
//!
//! * **Patrol.** Every so often the daemon looks at a project and, if anything is stuck, unresolved or
//!   loose (a paused mission, a mission waiting on approval or on its merge, a mission that has gone
//!   quiet, a job that just failed), wakes the manager with exactly that list. A project with nothing to
//!   handle costs nothing: no wake, no turn.
//! * **Goals.** When a project has nothing running and its manager has been idle for a while, a
//!   `goals.md` in the project's memory (written by the operator, or by the manager itself) is put in
//!   front of it with the instruction to pick the next piece of work and start it. No goals file, no
//!   prompt: the daemon cannot invent what to build.
//!
//! Both are deliberately rate-limited so an unfixable situation never turns into an endless loop of
//! turns that each cost money.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tendril_core::missions::model::{MissionFile, MissionState};
use tendril_core::models::{JobItem, JobStatus};

/// How often a project is looked at.
pub const PATROL_EVERY: Duration = Duration::from_secs(20 * 60);
/// A manager that took a turn this recently is already on it.
pub const RECENTLY_ACTIVE: Duration = Duration::from_secs(10 * 60);
/// The same list of findings is raised at most this many times in a row; after that the manager is left
/// alone until something changes.
pub const MAX_REPEATS: u32 = 3;
/// How long a project with nothing running and an idle manager waits before the goals prompt.
pub const GOALS_AFTER_IDLE: chrono::Duration = chrono::Duration::minutes(60);
/// Without a new mission appearing, the goals prompt is not repeated for this long.
pub const GOALS_REPEAT_AFTER: Duration = Duration::from_secs(6 * 3600);
/// How much of `goals.md` goes into the prompt.
const GOALS_CHARS: usize = 4000;

fn minutes(d: chrono::Duration) -> i64 {
    d.num_minutes().max(0)
}

/// One thing that needs a manager's attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// What it is, without how long it has been so: the same problem twenty minutes later has the same
    /// key, which is what lets [`PatrolState::should_raise`] stop repeating it.
    pub key: String,
    /// The line the manager reads.
    pub text: String,
}

/// What needs a manager's attention in a project right now, one line each. Empty means nothing does.
pub fn findings(
    missions: &[MissionFile],
    recent_failed_jobs: &[JobItem],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for f in missions {
        let m = &f.mission;
        let age = minutes(now - m.updated);
        // A mission that paused again for a different reason is a different problem.
        let mut found = |text: String| {
            let reason = m.pause_reason.as_deref().map(str::trim).unwrap_or_default();
            out.push(Finding { key: format!("mission {} {} {reason}", f.id, m.state.as_str()), text })
        };
        match m.state {
            MissionState::Paused if age >= 5 => found(format!(
                "mission {} \"{}\" has been paused for {age} min{}",
                f.id,
                m.title,
                m.pause_reason
                    .as_deref()
                    .filter(|r| !r.trim().is_empty())
                    .map(|r| format!(" ({})", r.trim()))
                    .unwrap_or_default()
            )),
            MissionState::AwaitingApproval if age >= 5 => {
                found(format!("mission {} \"{}\" has been waiting for approval for {age} min", f.id, m.title))
            }
            MissionState::Review if age >= 15 => found(format!(
                "mission {} \"{}\" has sat in review for {age} min: get its work merged locally, then have a worker push the branch and open a pull request, watch its CI, and close the mission",
                f.id, m.title
            )),
            MissionState::Planning | MissionState::Running | MissionState::Validating if age >= 45 => found(format!(
                "mission {} \"{}\" is {} but has shown no activity for {age} min",
                f.id,
                m.title,
                m.state.as_str().to_ascii_lowercase()
            )),
            _ => {}
        }
    }
    let failed: Vec<&JobItem> = recent_failed_jobs.iter().filter(|j| matches!(j.status, JobStatus::Failed | JobStatus::Timeout)).collect();
    if !failed.is_empty() {
        let ids = failed.iter().take(5).map(|j| format!("{} ({})", j.id, j.job_type)).collect::<Vec<_>>().join(", ");
        let mut all: Vec<&str> = failed.iter().map(|j| j.id.as_str()).collect();
        all.sort_unstable();
        out.push(Finding {
            key: format!("failed jobs {}", all.join(",")),
            text: format!("{} job{} failed or timed out in the last hour: {ids}", failed.len(), if failed.len() == 1 { "" } else { "s" }),
        });
    }
    out
}

/// The wake message for a patrol that found something.
pub fn patrol_message(findings: &[Finding]) -> String {
    let list = findings.iter().map(|f| format!("- {}", f.text)).collect::<Vec<_>>().join("\n");
    format!(
        "Patrol: nobody has asked you anything, but these need handling:\n{list}\n\n\
Deal with each one now, yourself or through a worker, the way your standing orders say: unstick pauses, \
approve and run what is waiting, get finished work merged and closed, follow up failures with a sharper attempt. \
Do not wait for the operator. When it is all handled, reply with one short line saying what you did. \
If one of them really is the operator's to decide (a budget ran out, they paused it on purpose), say which and why in that line, \
once: an unchanged list is raised {MAX_REPEATS} times and then dropped until something changes."
    )
}

/// What the manager is told when a project has gone quiet and has standing goals.
pub fn goals_message(goals: &str) -> String {
    let goals: String = goals.trim().chars().take(GOALS_CHARS).collect();
    format!(
        "Nothing is running in this project and you have been idle for a while. This is the Goals memory, with what the operator asked for:\n\n{goals}\n\n\
Go through every open request. Check where it really stands against its \"done when\" (the repository, its pull request, \
finished and failed work), not where the last report said it was. For each one that is not finished, take the next step and start it: \
a mission or a plan, with a worker doing the engineering. Move what you have verified to Done and update the memory. \
Say in one line what you started. If every request is genuinely done and checked, say so in one line and stop."
    )
}

/// Per-project memory of what has already been raised, so nothing nags in a loop.
#[derive(Default)]
pub struct PatrolState {
    last_patrol: HashMap<String, Instant>,
    /// The last findings raised, and how many times in a row.
    raised: HashMap<String, (String, u32)>,
    /// When the goals prompt last went out, and how many missions existed then.
    goals_sent: HashMap<String, (Instant, usize)>,
}

impl PatrolState {
    /// Whether this project is due to be looked at, recording that it was.
    pub fn due(&mut self, project: &str, clock: Instant) -> bool {
        match self.last_patrol.get(project) {
            Some(at) if clock.duration_since(*at) < PATROL_EVERY => false,
            _ => {
                self.last_patrol.insert(project.to_string(), clock);
                true
            }
        }
    }

    /// Whether to raise these findings, counting repeats of an unchanged list. Unchanged means the same
    /// problems, however much older they have grown.
    pub fn should_raise(&mut self, project: &str, findings: &[Finding]) -> bool {
        if findings.is_empty() {
            self.raised.remove(project);
            return false;
        }
        let key = findings.iter().map(|f| f.key.as_str()).collect::<Vec<_>>().join("\n");
        let entry = self.raised.entry(project.to_string()).or_insert((String::new(), 0));
        if entry.0 == key {
            if entry.1 >= MAX_REPEATS {
                return false;
            }
            entry.1 += 1;
        } else {
            *entry = (key, 1);
        }
        true
    }

    /// Whether the goals prompt should go out now.
    pub fn goals_due(&mut self, project: &str, missions_total: usize, clock: Instant) -> bool {
        match self.goals_sent.get(project) {
            // Already sent and nothing was started since: leave it for a long while.
            Some((at, count)) if *count == missions_total && clock.duration_since(*at) < GOALS_REPEAT_AFTER => false,
            // Sent recently, even if something was started: let that work run.
            Some((at, _)) if clock.duration_since(*at) < Duration::from_secs(3600) => false,
            _ => {
                self.goals_sent.insert(project.to_string(), (clock, missions_total));
                true
            }
        }
    }
}

/// Whether the project has any live (non-terminal, not merely waiting-for-review-merge) work.
pub fn has_live_work(missions: &[MissionFile], running_jobs: usize) -> bool {
    running_jobs > 0
        || missions.iter().any(|f| {
            matches!(
                f.mission.state,
                MissionState::Planning | MissionState::Running | MissionState::Validating | MissionState::AwaitingApproval | MissionState::Paused | MissionState::Review
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mission(id: &str, state: MissionState, minutes_ago: i64, reason: Option<&str>) -> MissionFile {
        let now = chrono::Utc::now();
        let mut value = serde_json::json!({
            "id": id,
            "folderName": format!("{id}-x"),
            "folderPath": "/tmp/x",
            "title": format!("Mission {id}"),
            "goal": "g",
            "project": "Acme",
            "state": state.as_str(),
            "created": now,
            "updated": now - chrono::Duration::minutes(minutes_ago),
        });
        if let Some(r) = reason {
            value["pauseReason"] = r.into();
        }
        serde_json::from_value(value).unwrap()
    }

    fn finding(text: &str) -> Finding {
        Finding { key: text.to_string(), text: text.to_string() }
    }

    fn failed_job(id: &str) -> JobItem {
        serde_json::from_value(serde_json::json!({
            "id": id, "type": "ExecutePlan", "planFile": "", "project": "Acme", "status": "Failed", "provider": "claude",
        }))
        .unwrap()
    }

    #[test]
    fn a_quiet_project_has_nothing_to_patrol() {
        let now = chrono::Utc::now();
        assert!(findings(&[], &[], now).is_empty());
        // Healthy running work and fresh pauses are left alone.
        let ms = [mission("1", MissionState::Running, 3, None), mission("2", MissionState::Paused, 1, Some("x")), mission("3", MissionState::Completed, 900, None)];
        assert!(findings(&ms, &[], now).is_empty());
    }

    #[test]
    fn stuck_loose_and_quiet_work_is_found_with_reasons() {
        let now = chrono::Utc::now();
        let ms = [
            mission("1", MissionState::Paused, 30, Some("No agent output for 10 minutes")),
            mission("2", MissionState::AwaitingApproval, 10, None),
            mission("3", MissionState::Review, 40, None),
            mission("4", MissionState::Running, 90, None),
            mission("5", MissionState::Review, 5, None),
        ];
        // A few seconds on, so the ages the missions were built with are comfortably reached.
        let found = findings(&ms, &[failed_job("00481")], now + chrono::Duration::seconds(5));
        assert_eq!(found.len(), 5, "{found:?}");
        assert!(found[0].text.contains("paused for 30 min") && found[0].text.contains("No agent output"), "{found:?}");
        assert!(found[1].text.contains("waiting for approval"));
        assert!(found[2].text.contains("sat in review"));
        assert!(found[3].text.contains("no activity for 90 min"));
        assert!(found[4].text.contains("00481"));
    }

    #[test]
    fn the_patrol_message_lists_everything_and_says_not_to_wait() {
        let text = patrol_message(&[finding("a"), finding("b")]);
        assert!(text.contains("- a\n- b"));
        assert!(text.contains("Do not wait for the operator"));
    }

    #[test]
    fn a_problem_that_only_grew_older_is_the_same_problem() {
        let now = chrono::Utc::now() + chrono::Duration::seconds(5);
        let ms = [mission("1", MissionState::Paused, 30, Some("Cost budget reached"))];
        let jobs = [failed_job("00481")];
        let mut s = PatrolState::default();
        for round in 0..MAX_REPEATS as i64 {
            let found = findings(&ms, &jobs, now + chrono::Duration::minutes(20 * round));
            assert!(found[0].text.contains(&format!("paused for {} min", 30 + 20 * round)), "{found:?}");
            assert!(s.should_raise("A", &found), "round {round}");
        }
        let later = findings(&ms, &jobs, now + chrono::Duration::minutes(20 * MAX_REPEATS as i64));
        assert!(!s.should_raise("A", &later), "the age in the text must not make it look new");
        // It moving to another state is new.
        let moved = [mission("1", MissionState::Review, 30, None)];
        assert!(s.should_raise("A", &findings(&moved, &jobs, now)));
    }

    #[test]
    fn a_project_is_looked_at_once_per_interval() {
        let mut s = PatrolState::default();
        let t = Instant::now();
        assert!(s.due("A", t));
        assert!(!s.due("A", t + Duration::from_secs(60)));
        assert!(s.due("B", t));
        assert!(s.due("A", t + PATROL_EVERY + Duration::from_secs(1)));
    }

    #[test]
    fn an_unchanged_problem_is_raised_a_few_times_then_dropped_until_it_changes() {
        let mut s = PatrolState::default();
        let same = vec![finding("mission 1 paused")];
        for _ in 0..MAX_REPEATS {
            assert!(s.should_raise("A", &same));
        }
        assert!(!s.should_raise("A", &same), "no endless nagging");
        assert!(s.should_raise("A", &[finding("mission 2 paused")]), "a new problem is raised");
        assert!(!s.should_raise("A", &[]), "nothing found, nothing raised");
        assert!(s.should_raise("A", &same), "after it cleared, a return is new");
    }

    #[test]
    fn goals_are_raised_once_then_only_when_work_was_started_or_a_long_time_passed() {
        let mut s = PatrolState::default();
        let t = Instant::now();
        assert!(s.goals_due("A", 4, t));
        assert!(!s.goals_due("A", 4, t + Duration::from_secs(2 * 3600)), "nothing started: wait");
        assert!(!s.goals_due("A", 5, t + Duration::from_secs(600)), "work started: let it run");
        assert!(s.goals_due("A", 5, t + Duration::from_secs(3 * 3600)), "it ran and finished: next goal");
        assert!(s.goals_due("A", 5, t + Duration::from_secs(10 * 3600)), "or a long wait");
    }

    #[test]
    fn live_work_means_something_in_flight_or_waiting() {
        assert!(!has_live_work(&[], 0));
        assert!(has_live_work(&[], 1));
        assert!(has_live_work(&[mission("1", MissionState::Running, 1, None)], 0));
        assert!(!has_live_work(&[mission("1", MissionState::Completed, 1, None), mission("2", MissionState::Cancelled, 1, None)], 0));
        let msg = goals_message("  Ship the trader walkthrough  ");
        assert!(msg.contains("Ship the trader walkthrough") && msg.contains("start it"));
    }
}
