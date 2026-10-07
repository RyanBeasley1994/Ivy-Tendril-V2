//! What wakes a project's manager when nobody has messaged it.
//!
//! A manager only runs a turn when it is prompted, so two things have to prompt it:
//!
//! * **Mission state changes.** A mission that needs its approval, pauses, reaches review, completes
//!   or is cancelled is the manager's business. The jobs a mission runs inside itself are not: the
//!   mission driver handles those, and waking the manager for each one only buys a paragraph saying
//!   nothing needs doing.
//! * **Wake-ups it asked for.** `tendril manager wake --in 20m --note "..."` is how a manager keeps a
//!   promise to look at something later (a CI run, a deploy) instead of just saying it will.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tendril_core::chat::manager_brief::manager_session_id;
use tendril_core::missions::model::MissionState;
use tendril_core::missions::store::list_missions;
use tokio::sync::Mutex;

const TICK: Duration = Duration::from_secs(10);
/// A wake-up shorter than this is a busy loop in disguise; longer than a week is a forgotten one.
pub const MIN_WAKE_SECONDS: u64 = 30;
pub const MAX_WAKE_SECONDS: u64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wake {
    pub id: String,
    pub project: String,
    /// RFC 3339.
    pub due_at: String,
    pub note: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WakeFile {
    wakes: Vec<Wake>,
}

/// One writer at a time: the route adds, the scheduler removes.
static FILE_LOCK: Mutex<()> = Mutex::const_new(());

fn wakes_path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("manager-wakes.json")
}

fn read_wakes(tendril_home: &Path) -> WakeFile {
    std::fs::read_to_string(wakes_path(tendril_home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_wakes(tendril_home: &Path, file: &WakeFile) -> std::io::Result<()> {
    std::fs::write(wakes_path(tendril_home), serde_json::to_vec_pretty(file).unwrap_or_default())
}

/// Schedules a wake-up and returns it.
pub async fn add_wake(
    tendril_home: &Path,
    project: &str,
    after_seconds: u64,
    note: &str,
) -> std::io::Result<Wake> {
    let seconds = after_seconds.clamp(MIN_WAKE_SECONDS, MAX_WAKE_SECONDS);
    let wake = Wake {
        id: uuid::Uuid::new_v4().to_string(),
        project: project.to_string(),
        due_at: (chrono::Utc::now() + chrono::Duration::seconds(seconds as i64)).to_rfc3339(),
        note: note.trim().to_string(),
    };
    let _guard = FILE_LOCK.lock().await;
    let mut file = read_wakes(tendril_home);
    file.wakes.push(wake.clone());
    write_wakes(tendril_home, &file)?;
    Ok(wake)
}

/// Takes every wake that is due out of the file and returns them.
async fn take_due(tendril_home: &Path) -> Vec<Wake> {
    let _guard = FILE_LOCK.lock().await;
    let mut file = read_wakes(tendril_home);
    let now = chrono::Utc::now();
    let (due, later): (Vec<Wake>, Vec<Wake>) = file.wakes.drain(..).partition(|w| {
        chrono::DateTime::parse_from_rfc3339(&w.due_at)
            .map(|d| d.with_timezone(&chrono::Utc) <= now)
            .unwrap_or(true)
    });
    if !due.is_empty() {
        file.wakes = later;
        let _ = write_wakes(tendril_home, &file);
    }
    due
}

/// The mission states a manager has to react to.
fn is_manager_business(state: MissionState) -> bool {
    matches!(
        state,
        MissionState::AwaitingApproval
            | MissionState::Paused
            | MissionState::Review
            | MissionState::Completed
            | MissionState::Cancelled
    )
}

fn describe(state: MissionState) -> &'static str {
    match state {
        MissionState::AwaitingApproval => "has its milestones planned and is waiting for approval (`tendril mission approve <id>`)",
        MissionState::Paused => "is paused",
        MissionState::Review => "passed validation and is in review, ready to merge",
        MissionState::Completed => "is completed",
        MissionState::Cancelled => "was cancelled",
        _ => "changed state",
    }
}

async fn manager_exists(state: &AppState, project: &str) -> Option<String> {
    let id = manager_session_id(project);
    state.chat_manager.get_session(&id).await.ok().map(|_| id)
}

/// The prompt that starts a morning briefing turn.
const BRIEFING_PROMPT: &str = "It is briefing time. In two to four short sentences, in your normal voice, tell the operator what finished since yesterday, what is stuck or waiting on them, what you are doing next, and roughly what it has cost. If nothing happened, say so in one line.";
/// A briefing missed by more than this (the daemon was down at the time) is skipped, not sent late.
const BRIEFING_GRACE_MINUTES: i64 = 180;
/// A project whose manager has been quiet this long gets no briefing.
const BRIEFING_ACTIVE_WITHIN_HOURS: i64 = 36;

fn briefing_state_path(home: &Path) -> PathBuf {
    home.join("manager-briefing-state.json")
}

/// Whether a briefing is due now: the wall clock (UTC plus the configured offset) is past the chosen
/// time, within the grace window, and it has not been sent today.
pub fn briefing_due(
    cfg: &crate::push::BriefingConfig,
    now_utc: chrono::DateTime<chrono::Utc>,
    last_sent_date: Option<&str>,
) -> Option<String> {
    let local = now_utc + chrono::Duration::minutes(cfg.utc_offset_minutes as i64);
    let date = local.format("%Y-%m-%d").to_string();
    if last_sent_date == Some(date.as_str()) {
        return None;
    }
    let (h, m) = cfg.time.split_once(':')?;
    let target = chrono::NaiveTime::from_hms_opt(h.trim().parse().ok()?, m.trim().parse().ok()?, 0)?;
    let minutes_past = (local.time() - target).num_minutes();
    (0..=BRIEFING_GRACE_MINUTES).contains(&minutes_past).then_some(date)
}

async fn latest_reply(state: &AppState, manager: &str) -> Option<tendril_core::chat::ChatSession> {
    state.chat_manager.get_session(manager).await.ok()
}

/// A manager's turn just ended: decide whether that is worth a push, and send it.
async fn on_turn_end(state: &AppState, project: &str, briefing: bool) {
    let cfg = crate::push::PushConfig::load(&state.tendril_home);
    if !cfg.enabled() {
        return;
    }
    let manager = manager_session_id(project);
    let Some(session) = latest_reply(state, &manager).await else { return };
    let Some(idx) = session.messages.iter().rposition(|m| m.role == "assistant" && !m.content.trim().is_empty()) else {
        return;
    };
    let reply = &session.messages[idx];
    let started_by_user = idx > 0 && session.messages[idx - 1].role == "user";
    let seconds = (chrono::Utc::now() - reply.timestamp).num_seconds();
    // A turn the operator typed and watched finish tells them nothing new.
    if !briefing && started_by_user && seconds < crate::push::WATCHED_TURN_SECONDS {
        return;
    }

    let text = crate::push::plain(&reply.content);
    if briefing {
        let _ = crate::push::send(&cfg, &format!("{project} · morning briefing"), &crate::push::clip(&text, 900), false).await;
        return;
    }

    let missions: Vec<_> = list_missions(&state.mission_driver.paths().missions_dir)
        .into_iter()
        .filter(|f| f.mission.project.eq_ignore_ascii_case(project))
        .collect();
    let waiting: Vec<_> = missions
        .iter()
        .filter(|f| matches!(f.mission.state, MissionState::AwaitingApproval | MissionState::Paused))
        .collect();
    let running_jobs = state
        .job_manager
        .list_non_terminal_jobs()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|j| j.project.eq_ignore_ascii_case(project))
        .filter(|j| matches!(j.status, tendril_core::models::JobStatus::Running | tendril_core::models::JobStatus::Queued))
        .count();
    let still_working = running_jobs > 0
        || missions.iter().any(|f| {
            matches!(f.mission.state, MissionState::Planning | MissionState::Running | MissionState::Validating)
        });

    match crate::push::classify_turn_end(&reply.content, waiting.len(), still_working) {
        Some(crate::push::TurnOutcome::NeedsYou) => {
            let asked = crate::push::last_paragraph(&reply.content).ends_with('?');
            let body = if asked {
                crate::push::clip(&crate::push::plain(crate::push::last_paragraph(&reply.content)), 300)
            } else {
                waiting
                    .first()
                    .map(|f| {
                        let what = if f.mission.state == MissionState::Paused { "is paused" } else { "is waiting for approval" };
                        format!("\"{}\" {what}.", f.mission.title)
                    })
                    .unwrap_or_else(|| crate::push::clip(&text, 300))
            };
            let _ = crate::push::send(&cfg, &format!("{project} · needs you"), &body, true).await;
        }
        Some(crate::push::TurnOutcome::WorkComplete) => {
            let _ = crate::push::send(&cfg, &format!("{project} · work complete"), &crate::push::clip(&text, 300), false).await;
        }
        None => {}
    }
}

fn project_repo_dir(state: &AppState, project: &str) -> Option<PathBuf> {
    let settings = tendril_core::config::load_config(&state.config_path).ok()?;
    let home = state.tendril_home.to_string_lossy().to_string();
    settings
        .projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(project))
        .and_then(|p| p.repos.first())
        .map(|r| PathBuf::from(tendril_core::config::expand_variables(&r.path, &home)))
}

pub fn spawn_manager_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut seen: Option<HashMap<String, MissionState>> = None;
        let mut was_working: HashMap<String, bool> = HashMap::new();
        let mut briefing_pending: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;

            // 1. Mission state changes. The first pass only records where things stand, so a daemon
            //    restart does not announce every mission that is already waiting.
            let current: HashMap<String, (MissionState, String, String, Option<String>)> =
                list_missions(&state.mission_driver.paths().missions_dir)
                    .into_iter()
                    .map(|f| {
                        (
                            f.id.clone(),
                            (
                                f.mission.state,
                                f.mission.project.clone(),
                                f.mission.title.clone(),
                                f.mission.pause_reason.clone(),
                            ),
                        )
                    })
                    .collect();
            if let Some(previous) = &seen {
                for (id, (now_state, project, title, reason)) in &current {
                    let changed = previous.get(id).map(|p| p != now_state).unwrap_or(true);
                    if !changed || !is_manager_business(*now_state) {
                        continue;
                    }
                    let Some(manager) = manager_exists(&state, project).await else { continue };
                    let mut message = format!("Mission {id} \"{title}\" {}.", describe(*now_state));
                    if *now_state == MissionState::Paused {
                        if let Some(reason) = reason.as_deref().filter(|r| !r.trim().is_empty()) {
                            message.push_str(&format!(" Reason: {}.", reason.trim()));
                        }
                    }
                    if let Err(e) = state.chat_manager.notify_event(&manager, &message).await {
                        tracing::debug!("Could not wake manager {manager}: {e}");
                    }
                }
            }
            seen = Some(current.into_iter().map(|(id, (s, ..))| (id, s)).collect());

            // 2. Wake-ups the managers scheduled for themselves.
            for wake in take_due(&state.tendril_home).await {
                let Some(manager) = manager_exists(&state, &wake.project).await else { continue };
                let message = format!(
                    "Wake-up you scheduled for yourself: {}",
                    if wake.note.is_empty() { "(no note)" } else { &wake.note }
                );
                if let Err(e) = state.chat_manager.notify_event(&manager, &message).await {
                    tracing::debug!("Could not wake manager {manager}: {e}");
                }
            }

            // 3. Pull requests a manager asked to have watched.
            for watch in crate::pr_watch::due_watches(&state.tendril_home).await {
                let Some(dir) = project_repo_dir(&state, &watch.project) else {
                    crate::pr_watch::remove_watch(&state.tendril_home, &watch.id).await;
                    continue;
                };
                let age = chrono::DateTime::parse_from_rfc3339(&watch.created_at)
                    .map(|d| (chrono::Utc::now() - d.with_timezone(&chrono::Utc)).num_seconds())
                    .unwrap_or(0);
                match crate::pr_watch::fetch(&watch, &dir).await {
                    Ok(status) => {
                        if let Some(message) = crate::pr_watch::wake_message(&watch, &status, age) {
                            crate::pr_watch::remove_watch(&state.tendril_home, &watch.id).await;
                            if let Some(manager) = manager_exists(&state, &watch.project).await {
                                if let Err(e) = state.chat_manager.notify_event(&manager, &message).await {
                                    tracing::debug!("Could not wake manager {manager}: {e}");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("PR watch {} #{}: {e}", watch.project, watch.pr);
                        // A PR that does not exist will not start to: stop asking after the give-up time.
                        if age >= crate::pr_watch::GIVE_UP_AFTER_SECONDS {
                            crate::pr_watch::remove_watch(&state.tendril_home, &watch.id).await;
                        }
                    }
                }
            }

            // 4. A manager's turn ended: tell the operator if it needs them or the work is complete.
            let settings = tendril_core::config::load_config(&state.config_path).unwrap_or_default();
            for project in settings.projects.iter().map(|p| p.name.clone()) {
                let id = manager_session_id(&project);
                let working = state.chat_manager.is_generating(&id).await;
                let before = was_working.insert(project.clone(), working);
                if before == Some(true) && !working {
                    let briefing = briefing_pending.remove(&project);
                    on_turn_end(&state, &project, briefing).await;
                }
            }

            // 5. The morning briefing.
            let push_cfg = crate::push::PushConfig::load(&state.tendril_home);
            if let Some(cfg) = &push_cfg.briefing {
                let last: Option<String> = std::fs::read_to_string(briefing_state_path(&state.tendril_home))
                    .ok()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                    .and_then(|v| v["lastDate"].as_str().map(str::to_string));
                if let Some(date) = briefing_due(cfg, chrono::Utc::now(), last.as_deref()) {
                    let _ = std::fs::write(
                        briefing_state_path(&state.tendril_home),
                        serde_json::json!({ "lastDate": date }).to_string(),
                    );
                    for project in settings.projects.iter().map(|p| p.name.clone()) {
                        let Some(manager) = manager_exists(&state, &project).await else { continue };
                        let recent = latest_reply(&state, &manager)
                            .await
                            .map(|s| (chrono::Utc::now() - s.updated_at).num_hours() < BRIEFING_ACTIVE_WITHIN_HOURS)
                            .unwrap_or(false);
                        if !recent {
                            continue;
                        }
                        briefing_pending.insert(project.clone());
                        if let Err(e) = state.chat_manager.notify_event(&manager, BRIEFING_PROMPT).await {
                            tracing::debug!("Could not start the briefing for {project}: {e}");
                            briefing_pending.remove(&project);
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tendril-wakes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_wake_is_clamped_and_only_comes_due_when_its_time_arrives() {
        let home = temp_home();
        // Asked for one second: clamped up, so it is not due yet.
        let wake = add_wake(&home, "Acme", 1, "check CI").await.unwrap();
        let due_at = chrono::DateTime::parse_from_rfc3339(&wake.due_at).unwrap();
        assert!(due_at > chrono::Utc::now() + chrono::Duration::seconds(20));
        assert!(take_due(&home).await.is_empty());
        assert_eq!(read_wakes(&home).wakes.len(), 1, "an undue wake stays scheduled");

        // One already past is taken exactly once, and the later one is kept.
        let mut file = read_wakes(&home);
        file.wakes.push(Wake {
            id: "past".into(),
            project: "Acme".into(),
            due_at: (chrono::Utc::now() - chrono::Duration::seconds(5)).to_rfc3339(),
            note: "look at the deploy".into(),
        });
        write_wakes(&home, &file).unwrap();
        let due = take_due(&home).await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].note, "look at the deploy");
        assert!(take_due(&home).await.is_empty());
        assert_eq!(read_wakes(&home).wakes.len(), 1);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn only_the_states_a_manager_must_act_on_wake_it() {
        assert!(is_manager_business(MissionState::AwaitingApproval));
        assert!(is_manager_business(MissionState::Paused));
        assert!(is_manager_business(MissionState::Review));
        assert!(!is_manager_business(MissionState::Running));
        assert!(!is_manager_business(MissionState::Validating));
        assert!(!is_manager_business(MissionState::Planning));
    }

    #[test]
    fn a_briefing_is_due_once_a_day_inside_its_grace_window() {
        use crate::push::BriefingConfig;
        let cfg = BriefingConfig { time: "08:00".into(), utc_offset_minutes: 60 };
        let at = |h: u32, m: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 7, h, m, 0).unwrap();
        // 07:30 UTC is 08:30 local: due, and it names today's local date.
        assert_eq!(briefing_due(&cfg, at(7, 30), None).as_deref(), Some("2026-10-07"));
        // Already sent today.
        assert_eq!(briefing_due(&cfg, at(7, 30), Some("2026-10-07")), None);
        // Before the time, and long after it (the daemon was down): not due.
        assert_eq!(briefing_due(&cfg, at(6, 30), None), None);
        assert_eq!(briefing_due(&cfg, at(12, 0), None), None);
        // A bad time never fires.
        let bad = BriefingConfig { time: "soon".into(), utc_offset_minutes: 0 };
        assert_eq!(briefing_due(&bad, at(8, 0), None), None);
    }
}
