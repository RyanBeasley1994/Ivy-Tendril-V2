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

pub fn spawn_manager_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut seen: Option<HashMap<String, MissionState>> = None;
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
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
