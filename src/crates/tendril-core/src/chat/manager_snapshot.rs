//! Where a project stands, handed to its manager at the top of every turn.
//!
//! Without it a manager's first move on every wake is to find out what is going on: list the missions,
//! list the jobs, list its tasks, look at the pull requests. Each of those is a tool call that sends the
//! whole turn so far back through the model, and the daemon already knows every answer. So it says them
//! once, in a few lines, and the manager only looks further at what it is about to act on.

use crate::chat::manager_brief::manager_session_id;
use crate::missions::model::MissionState;
use crate::missions::store::{list_missions, missions_dir};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;

/// Missions shown at most; a project with more live ones than this has `tendril mission list`.
const MAX_MISSIONS: usize = 12;

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(Value::Null)
}

fn ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let minutes = (now - at).num_minutes().max(0);
    match minutes {
        0..=89 => format!("{minutes} min ago"),
        90..=2879 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

fn parse(at: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at.as_str()?).ok().map(|d| d.with_timezone(&Utc))
}

/// The project a manager session belongs to, by the session id every manager is given.
pub fn project_of(tendril_home: &Path, session_id: &str) -> Option<String> {
    let settings = crate::config::load_config(&crate::config::get_config_path(tendril_home)).ok()?;
    settings.projects.iter().map(|p| p.name.clone()).find(|name| manager_session_id(name) == session_id)
}

/// The snapshot for `project`, as a prompt section.
pub fn snapshot(tendril_home: &Path, project: &str, now: DateTime<Utc>) -> String {
    let mine = |v: &Value| v["project"].as_str().is_some_and(|p| p.eq_ignore_ascii_case(project));
    let mut lines: Vec<String> = Vec::new();

    let live: Vec<_> = list_missions(&missions_dir(tendril_home))
        .into_iter()
        .filter(|f| f.mission.project.eq_ignore_ascii_case(project))
        .filter(|f| !matches!(f.mission.state, MissionState::Completed | MissionState::Cancelled))
        .collect();
    if !live.is_empty() {
        lines.push("Missions not yet closed:".into());
        for f in live.iter().take(MAX_MISSIONS) {
            let m = &f.mission;
            let done = m.milestones.len();
            let reason = m.pause_reason.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(|r| format!(" ({r})")).unwrap_or_default();
            lines.push(format!(
                "- {} \"{}\": {}{reason}, {done} milestone{}, last moved {}{}",
                f.id,
                m.title,
                m.state.as_str().to_ascii_lowercase(),
                if done == 1 { "" } else { "s" },
                ago(m.updated, now),
                m.branch.as_deref().map(|b| format!(", branch {b}")).unwrap_or_default(),
            ));
        }
        if live.len() > MAX_MISSIONS {
            lines.push(format!("- and {} more (`tendril mission list --project \"{project}\"`)", live.len() - MAX_MISSIONS));
        }
    }

    let tasks = read_json(&tendril_home.join("manager-tasks.json"));
    let tasks: Vec<&Value> = tasks["tasks"].as_array().map(|a| a.iter().filter(|t| mine(t)).collect()).unwrap_or_default();
    if !tasks.is_empty() {
        lines.push("Tasks:".into());
        for t in tasks {
            let state = match parse(&t["finishedAt"]) {
                Some(at) => format!("finished {}, not cleaned up", ago(at, now)),
                None => format!("running, started {}", parse(&t["createdAt"]).map(|at| ago(at, now)).unwrap_or_default()),
            };
            lines.push(format!(
                "- {} \"{}\": {state}{}",
                t["id"].as_str().unwrap_or("?"),
                t["title"].as_str().unwrap_or(""),
                t["branch"].as_str().map(|b| format!(", branch {b}")).unwrap_or_default(),
            ));
        }
    }

    let watches = read_json(&tendril_home.join("manager-pr-watches.json"));
    let prs: Vec<String> = watches["watches"]
        .as_array()
        .map(|a| a.iter().filter(|w| mine(w)).filter_map(|w| w["pr"].as_u64()).map(|n| n.to_string()).collect())
        .unwrap_or_default();
    if !prs.is_empty() {
        lines.push(format!("Pull requests being watched for you (you are woken when their checks finish): {}", prs.join(", ")));
    }

    let wakes = read_json(&tendril_home.join("manager-wakes.json"));
    for w in wakes["wakes"].as_array().map(|a| a.iter().filter(|w| mine(w)).collect::<Vec<_>>()).unwrap_or_default() {
        let when = parse(&w["dueAt"]).map(|at| (at - now).num_minutes().max(0)).unwrap_or(0);
        lines.push(format!("Wake-up in {when} min: {}", w["note"].as_str().unwrap_or("")));
    }

    let body = if lines.is_empty() {
        "Nothing is in flight: no open missions, no tasks, no watched pull requests, no wake-ups scheduled.".to_string()
    } else {
        lines.join("\n")
    };
    format!(
        "# Where the project stands\nFrom the daemon, as of this turn. It is current: do not list missions, tasks or jobs again to confirm it; look further only at the thing you are about to act on.\n{body}\n---\n\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_project_says_so_in_one_line() {
        let home = tempfile::tempdir().unwrap();
        let text = snapshot(home.path(), "Acme", Utc::now());
        assert!(text.contains("Nothing is in flight"), "{text}");
        assert!(text.contains("do not list missions, tasks or jobs again"));
    }

    #[test]
    fn tasks_watches_and_wakes_of_this_project_are_listed_and_others_are_not() {
        let home = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let at = |m: i64| (now + chrono::Duration::minutes(m)).to_rfc3339();
        std::fs::write(
            home.path().join("manager-tasks.json"),
            serde_json::json!({"tasks": [
                {"id": "ab12", "project": "Acme", "title": "Rename flag", "branch": "tendril/task-ab12", "createdAt": at(-10)},
                {"id": "cd34", "project": "acme", "title": "Fix lint", "createdAt": at(-90), "finishedAt": at(-40)},
                {"id": "zz99", "project": "Other", "title": "Not ours", "createdAt": at(-1)},
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            home.path().join("manager-pr-watches.json"),
            serde_json::json!({"watches": [{"project": "Acme", "pr": 46}, {"project": "Other", "pr": 7}]}).to_string(),
        )
        .unwrap();
        std::fs::write(
            home.path().join("manager-wakes.json"),
            serde_json::json!({"wakes": [{"project": "Acme", "dueAt": at(14), "note": "deploy 123"}]}).to_string(),
        )
        .unwrap();
        let text = snapshot(home.path(), "Acme", now);
        assert!(text.contains("- ab12 \"Rename flag\": running, started 10 min ago, branch tendril/task-ab12"), "{text}");
        assert!(text.contains("- cd34 \"Fix lint\": finished 40 min ago, not cleaned up"), "{text}");
        assert!(!text.contains("zz99") && !text.contains(" 7"), "{text}");
        assert!(text.contains("checks finish): 46"), "{text}");
        assert!(text.contains("min: deploy 123"), "{text}");
    }
}
