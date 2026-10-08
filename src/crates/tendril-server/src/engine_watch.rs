//! Noticing that an engine is down, and telling the managers it is hurting.
//!
//! When the model behind an agent stops answering (a local model server that fell over, a login that
//! expired, a rate limit), every job on that agent starts, produces nothing, and is stopped by the
//! stale-output watchdog ten minutes later. Each mission those jobs belong to then pauses with "no agent
//! output". Resuming it just feeds it to the same dead engine, so the manager has to be told the pattern,
//! not each symptom: several silent jobs on one agent in a short time means the agent is sick, and the
//! fix is to move the project to another engine (the manager may; it is in its standing orders).

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};
use tendril_core::models::{JobItem, JobStatus};

/// How far back a silent job still counts as part of the same trouble.
pub const WINDOW: chrono::Duration = chrono::Duration::minutes(45);
/// Silent jobs on one agent, across all projects, before it is called sick. One is a fluke.
pub const THRESHOLD: usize = 2;
/// How long a manager is left alone after being told about an agent.
pub const REALERT_AFTER: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilentJob {
    pub id: String,
    pub project: String,
    pub provider: String,
}

/// The jobs that were stopped for producing no output, recently enough to matter.
pub fn silent_jobs(jobs: &[JobItem], now: chrono::DateTime<chrono::Utc>) -> Vec<SilentJob> {
    jobs.iter()
        .filter(|j| matches!(j.status, JobStatus::Timeout | JobStatus::Failed))
        .filter(|j| j.status_message.as_deref().is_some_and(|m| m.contains("No agent output")))
        .filter(|j| j.completed_at.is_some_and(|at| now - at <= WINDOW))
        .filter(|j| !j.project.trim().is_empty())
        .map(|j| SilentJob { id: j.id.clone(), project: j.project.clone(), provider: j.provider.clone() })
        .collect()
}

/// One manager to tell: its project, the sick agent, that project's silent jobs, and how many silent
/// jobs the agent had in all.
#[derive(Debug, PartialEq, Eq)]
pub struct Alert {
    pub project: String,
    pub provider: String,
    pub job_ids: Vec<String>,
    pub total: usize,
}

/// Which managers to alert now. An agent is sick once [`THRESHOLD`] silent jobs have piled up on it; each
/// affected project is told once per [`REALERT_AFTER`].
pub fn plan_alerts(
    silent: &[SilentJob],
    last_alert: &mut HashMap<(String, String), Instant>,
    clock: Instant,
) -> Vec<Alert> {
    let mut by_provider: BTreeMap<&str, Vec<&SilentJob>> = BTreeMap::new();
    for job in silent {
        by_provider.entry(&job.provider).or_default().push(job);
    }
    let mut out = Vec::new();
    for (provider, jobs) in by_provider {
        if jobs.len() < THRESHOLD {
            continue;
        }
        let mut by_project: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for job in &jobs {
            by_project.entry(&job.project).or_default().push(job.id.clone());
        }
        for (project, job_ids) in by_project {
            let key = (provider.to_string(), project.to_string());
            if last_alert.get(&key).is_some_and(|at| clock.duration_since(*at) < REALERT_AFTER) {
                continue;
            }
            last_alert.insert(key, clock);
            out.push(Alert { project: project.to_string(), provider: provider.to_string(), job_ids, total: jobs.len() });
        }
    }
    out
}

pub fn message(alert: &Alert) -> String {
    let ids = alert.job_ids.iter().map(|i| format!("job {i}")).collect::<Vec<_>>().join(", ");
    format!(
        "Engine trouble: {total} job{s} on {provider} went silent (no agent output) and were stopped in the last 45 minutes, {ids} in this project. \
When jobs on one agent go quiet together, the engine itself is down (for example a local model server that is not answering, an expired login, or a rate limit); the work is not what is wrong, and resuming a paused mission on it will only stall again. \
Move this project off {provider} now: `tendril manager engine --project {project} --planner claude --worker claude --judge claude --validator claude` (use another configured agent if claude is limited), then `tendril mission resume <id>` for each mission paused with no agent output (`tendril mission list --project {project} --state paused`). \
Tell the operator in one line what you switched and why, and that {provider} needs looking at. If the other agents are failing too, say so once and stop retrying.",
        total = alert.total,
        s = if alert.total == 1 { "" } else { "s" },
        provider = alert.provider,
        project = alert.project,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn job(id: &str, project: &str, provider: &str, minutes_ago: i64, message: &str, status: JobStatus) -> JobItem {
        let mut j: JobItem = serde_json::from_value(serde_json::json!({
            "id": id, "type": "ExecutePlan", "planFile": "", "project": project,
            "status": status, "provider": provider,
        }))
        .unwrap();
        j.completed_at = Some(Utc::now() - chrono::Duration::minutes(minutes_ago));
        j.status_message = Some(message.to_string());
        j
    }

    fn silent(id: &str, project: &str, provider: &str) -> SilentJob {
        SilentJob { id: id.into(), project: project.into(), provider: provider.into() }
    }

    #[test]
    fn only_recent_no_output_stops_count() {
        let now = Utc::now();
        let jobs = vec![
            job("1", "A", "codex", 5, "No agent output for 10 minutes (stale output timeout)", JobStatus::Timeout),
            job("2", "A", "codex", 90, "No agent output for 10 minutes (stale output timeout)", JobStatus::Timeout),
            job("3", "A", "codex", 5, "Tests failed", JobStatus::Failed),
            job("4", "B", "claude", 10, "No agent output for 10 minutes (stale output timeout)", JobStatus::Timeout),
        ];
        let found = silent_jobs(&jobs, now);
        assert_eq!(found, vec![silent("1", "A", "codex"), silent("4", "B", "claude")]);
    }

    #[test]
    fn one_silent_job_is_a_fluke_and_two_on_an_agent_is_an_outage() {
        let mut last = HashMap::new();
        let t = Instant::now();
        assert!(plan_alerts(&[silent("1", "A", "codex")], &mut last, t).is_empty());
        let alerts = plan_alerts(&[silent("1", "A", "codex"), silent("2", "B", "codex"), silent("3", "A", "claude")], &mut last, t);
        // Both projects hit by codex are told; the lone claude failure is not an outage.
        assert_eq!(alerts.len(), 2);
        assert!(alerts.iter().all(|a| a.provider == "codex" && a.total == 2));
        assert_eq!(alerts[0].project, "A");
        assert_eq!(alerts[1].project, "B");
    }

    #[test]
    fn a_manager_is_told_once_then_again_only_after_a_while() {
        let mut last = HashMap::new();
        let t = Instant::now();
        let jobs = [silent("1", "A", "codex"), silent("2", "A", "codex")];
        assert_eq!(plan_alerts(&jobs, &mut last, t).len(), 1);
        assert!(plan_alerts(&jobs, &mut last, t + Duration::from_secs(60)).is_empty());
        assert_eq!(plan_alerts(&jobs, &mut last, t + REALERT_AFTER + Duration::from_secs(1)).len(), 1);
    }

    #[test]
    fn the_message_names_the_agent_the_cure_and_the_jobs() {
        let text = message(&Alert { project: "Trading Platform".into(), provider: "codex".into(), job_ids: vec!["00477".into(), "00480".into()], total: 3 });
        assert!(text.contains("3 jobs on codex"), "{text}");
        assert!(text.contains("job 00477, job 00480"), "{text}");
        assert!(text.contains("tendril manager engine --project Trading Platform"), "{text}");
        assert!(text.contains("tendril mission resume"), "{text}");
    }
}
