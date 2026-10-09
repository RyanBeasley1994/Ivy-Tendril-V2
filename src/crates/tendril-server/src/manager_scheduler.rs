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

/// The mission states a manager has to react to. A cancelled mission is not one: somebody decided that,
/// there is nothing to do about it, and a turn spent saying "noted" is a turn wasted.
fn is_manager_business(state: MissionState) -> bool {
    matches!(
        state,
        MissionState::AwaitingApproval
            | MissionState::Paused
            | MissionState::Review
            | MissionState::Completed
    )
}

/// Why a manager is being told about a mission now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wakeup {
    /// The mission just entered the state.
    Changed,
    /// The daemon (re)started and found the mission already waiting.
    Catchup,
}

fn waits_on_manager(state: MissionState) -> bool {
    matches!(state, MissionState::Paused | MissionState::AwaitingApproval)
}

/// Whether, and why, to tell a manager about a mission this tick.
///
/// A state *change* is the usual reason. One more keeps a stuck mission from being forgotten: a daemon
/// that has just started tells the manager about whatever is already paused or awaiting approval (the
/// transition happened while nobody was listening). A mission still waiting after that is not raised
/// again from here: the patrol ([`crate::patrol`]) lists it, together with everything else that is
/// stuck, in one message and so one turn, and stops after a few unchanged repeats.
fn plan_wakeup(first_pass: bool, previous: Option<MissionState>, now: MissionState) -> Option<Wakeup> {
    if !is_manager_business(now) {
        return None;
    }
    if first_pass {
        return waits_on_manager(now).then_some(Wakeup::Catchup);
    }
    (previous != Some(now)).then_some(Wakeup::Changed)
}

fn wakeup_message(id: &str, title: &str, state: MissionState, reason: Option<&str>, why: Wakeup) -> String {
    let mut message = match why {
        Wakeup::Changed => format!("Mission {id} \"{title}\" {}.", describe(state)),
        Wakeup::Catchup => format!(
            "Mission {id} \"{title}\" {}. (You may have missed this: the daemon restarted while it waited.)",
            describe(state)
        ),
    };
    if state == MissionState::Paused {
        if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
            message.push_str(&format!(" Reason: {reason}."));
        }
        message.push_str(
            " Unstick it yourself unless the operator paused it on purpose or a budget ran out: see \"A paused mission is yours to unstick\" in your briefing.",
        );
    }
    message
}

fn describe(state: MissionState) -> &'static str {
    match state {
        MissionState::AwaitingApproval => "has its milestones planned and is waiting for approval (`tendril mission approve <id>`)",
        MissionState::Paused => "is paused",
        MissionState::Review => "passed validation and is in review: check it against what the operator asked for, then push it, open its pull request and close it, as your standing orders say",
        MissionState::Completed => "is completed: if the request it belongs to has more to do, take the next step; if that was the last of it, check the request's \"done when\" and close it in the Goals memory",
        _ => "changed state",
    }
}

/// The project's manager session, when it has one. Every wake goes through here, so this is also where
/// a manager is given the current briefing: one that is only ever woken by the daemon is never opened in
/// the app, and would otherwise keep the briefing it was created with.
async fn manager_exists(state: &AppState, project: &str) -> Option<String> {
    let id = manager_session_id(project);
    state.chat_manager.get_session(&id).await.ok()?;
    crate::routes::projects::refresh_briefing(state, project).await;
    Some(id)
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

/// The tool calls the agent harness refused during a turn, read from the turn's recorded stream.
fn refusals_in(raw_stream: &str) -> Vec<tendril_core::jobs::denials::PermissionDenial> {
    let lines: Vec<String> = raw_stream.lines().map(str::to_string).collect();
    tendril_core::jobs::denials::extract_permission_denials(&lines)
}

/// What a manager is told when its commands were refused: which ones, why, and what to do instead.
/// Without this it reports that its shell is "blocked", asks the operator to allow it (which they cannot
/// do), and the work stops there.
fn refusal_message(denials: &[tendril_core::jobs::denials::PermissionDenial]) -> String {
    let shown = tendril_core::jobs::denials::describe_denials(denials).join("; ");
    format!(
        "Your last turn was cut short: these tool calls were refused and did not run: {shown}. \
A command is refused when it is outside your allowed tools or is not one simple command: pipes (`|`), `&&`, `;`, `$(...)`, backticks, redirects (`>`, `2>&1`) and heredocs all count. \
Run each again as a single plain command (pass a long mission goal inline as one quoted `--goal` argument), use the CLI's own filters instead of piping, or delegate the work to a worker. \
Do not ask the operator to allow your shell: they cannot, and you do not need them to. Carry on with what you were asked."
    )
}

/// How many corrections a project's manager gets in a window, so a command that can never succeed is not
/// retried in a loop.
const MAX_CORRECTIONS: u32 = 2;
const CORRECTION_WINDOW: Duration = Duration::from_secs(30 * 60);

fn may_correct(
    history: &mut HashMap<String, (std::time::Instant, u32)>,
    project: &str,
    clock: std::time::Instant,
) -> bool {
    let entry = history.entry(project.to_string()).or_insert((clock, 0));
    if clock.duration_since(entry.0) >= CORRECTION_WINDOW {
        *entry = (clock, 0);
    }
    if entry.1 >= MAX_CORRECTIONS {
        return false;
    }
    entry.1 += 1;
    true
}

/// After a manager's turn: if the harness refused any of its commands, say so in the log and prompt the
/// manager to carry on without them.
async fn correct_refusals(
    state: &AppState,
    project: &str,
    history: &mut HashMap<String, (std::time::Instant, u32)>,
) {
    let manager = manager_session_id(project);
    let Some(session) = latest_reply(state, &manager).await else { return };
    let Some(reply) = session.messages.iter().rev().find(|m| m.role == "assistant" && m.raw_stream.is_some()) else {
        return;
    };
    // Only a turn that has just ended counts; an old refusal is not worth a prompt now.
    if (chrono::Utc::now() - reply.timestamp).num_minutes() > 10 {
        return;
    }
    let denials = refusals_in(reply.raw_stream.as_deref().unwrap_or_default());
    if denials.is_empty() {
        return;
    }
    let summary = tendril_core::jobs::denials::describe_denials(&denials).join("; ");
    tracing::warn!("Manager of {project}: tool calls refused: {summary}");
    if !may_correct(history, project, std::time::Instant::now()) {
        return;
    }
    if let Err(e) = state.chat_manager.notify_event(&manager, &refusal_message(&denials)).await {
        tracing::debug!("Could not prompt manager {manager}: {e}");
    }
}

/// One patrol pass over every project that has a manager.
async fn patrol(state: &AppState, patrol_state: &mut crate::patrol::PatrolState) {
    use crate::patrol;
    let settings = tendril_core::config::load_config(&state.config_path).unwrap_or_default();
    if settings.projects.is_empty() {
        return;
    }
    let now = chrono::Utc::now();
    let mut failed = Vec::new();
    for status in [tendril_core::models::JobStatus::Failed, tendril_core::models::JobStatus::Timeout] {
        if let Ok(jobs) = state.job_manager.list_jobs(Some(status), 40).await {
            failed.extend(jobs.into_iter().filter(|j| j.completed_at.is_some_and(|at| now - at <= chrono::Duration::hours(1))));
        }
    }
    let running = state.job_manager.list_non_terminal_jobs().await.unwrap_or_default();
    let all_missions = list_missions(&state.mission_driver.paths().missions_dir);

    for project in settings.projects.iter().map(|p| p.name.clone()) {
        let id = manager_session_id(&project);
        let Ok(session) = state.chat_manager.get_session(&id).await else { continue };
        if state.chat_manager.is_generating(&id).await {
            continue;
        }
        let idle_for = now - session.updated_at;
        if idle_for.to_std().unwrap_or_default() < patrol::RECENTLY_ACTIVE {
            continue;
        }
        if !patrol_state.due(&project, std::time::Instant::now()) {
            continue;
        }
        let missions: Vec<_> = all_missions.iter().filter(|f| f.mission.project.eq_ignore_ascii_case(&project)).cloned().collect();
        let project_failed: Vec<_> = failed.iter().filter(|j| j.project.eq_ignore_ascii_case(&project)).cloned().collect();
        let mut found = patrol::findings(&missions, &project_failed, now);
        // What the manager itself has left untidy: tasks not cleaned up, finished missions not shipped.
        let repo = project_repo_dir(state, &project);
        found.extend(crate::manager_tasks::loose_ends(state, &project, repo.as_deref(), &missions, now).await);

        let message = if !found.is_empty() {
            patrol_state.should_raise(&project, &found).then(|| patrol::patrol_message(&found))
        } else {
            patrol_state.should_raise(&project, &found);
            let live = patrol::has_live_work(
                &missions,
                running.iter().filter(|j| j.project.eq_ignore_ascii_case(&project)).count(),
            );
            let goals = std::fs::read_to_string(
                tendril_core::config::get_project_root_dir(&state.tendril_home, &project).join("Memory").join("goals.md"),
            )
            .unwrap_or_default();
            (!live
                && !goals.trim().is_empty()
                && idle_for >= patrol::GOALS_AFTER_IDLE
                && patrol_state.goals_due(&project, missions.len(), std::time::Instant::now()))
            .then(|| patrol::goals_message(&goals))
        };
        if let Some(message) = message {
            crate::routes::projects::refresh_briefing(state, &project).await;
            if let Err(e) = state.chat_manager.notify_event(&id, &message).await {
                tracing::debug!("Could not patrol manager {id}: {e}");
            }
        }
    }
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
        let mut engine_alerts: HashMap<(String, String), std::time::Instant> = HashMap::new();
        let mut corrections: HashMap<String, (std::time::Instant, u32)> = HashMap::new();
        let mut patrol_state = crate::patrol::PatrolState::default();
        let mut patrol_checked = std::time::Instant::now() - Duration::from_secs(3600);
        let mut engines_checked = std::time::Instant::now() - Duration::from_secs(3600);
        let mut briefing_pending: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;

            // 1. Mission state changes, plus the catch-up in `plan_wakeup` so a mission that paused while
            //    nobody was listening is not left stuck. One still stuck later is the patrol's (2c).
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
            let first_pass = seen.is_none();
            let previous_states = seen.clone().unwrap_or_default();
            // Everything that changed for one project this tick goes to its manager as one message, so
            // it is one turn: after a restart, or when several missions move together, a wake each
            // would make the manager orient itself over and over.
            let mut changed: std::collections::BTreeMap<&str, Vec<String>> = std::collections::BTreeMap::new();
            for (id, (now_state, project, title, reason)) in &current {
                let Some(why) = plan_wakeup(first_pass, previous_states.get(id).copied(), *now_state) else {
                    continue;
                };
                changed.entry(project.as_str()).or_default().push(wakeup_message(id, title, *now_state, reason.as_deref(), why));
            }
            for (project, messages) in changed {
                let Some(manager) = manager_exists(&state, project).await else { continue };
                if let Err(e) = state.chat_manager.notify_event(&manager, &messages.join("\n\n")).await {
                    tracing::debug!("Could not wake manager {manager}: {e}");
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

            // 2a. Tasks the managers handed straight to an agent: tell each manager when its worker stops.
            crate::manager_tasks::report_finished(&state).await;

            // 2b. An engine that has gone down shows up as several jobs stopped for silence at once; the
            //     managers it is hurting are told the pattern, not left to rediscover it job by job.
            if engines_checked.elapsed() >= Duration::from_secs(60) {
                engines_checked = std::time::Instant::now();
                // Any job that just failed on a rate limit puts its agent on cooldown, so what starts next
                // (plan jobs outside missions included) is covered by the fallback chain until it resets.
                if let Ok(failed) = state.job_manager.list_jobs(Some(tendril_core::models::JobStatus::Failed), 30).await {
                    let now = chrono::Utc::now();
                    for job in failed.iter().filter(|j| j.completed_at.is_some_and(|at| now - at <= chrono::Duration::minutes(10))) {
                        let texts = [job.reported_failure_reason.as_deref(), job.status_message.as_deref()];
                        if let Some(reason) = tendril_core::missions::rate_limit::detect(texts.into_iter().flatten()) {
                            let from = job.completed_at.unwrap_or(now);
                            tendril_core::agents::cooldown::note_rate_limited(&job.provider, tendril_core::missions::rate_limit::wait_for(&reason, 3, from));
                        }
                    }
                }
                if let Ok(jobs) = state.job_manager.list_jobs(Some(tendril_core::models::JobStatus::Timeout), 60).await {
                    let silent = crate::engine_watch::silent_jobs(&jobs, chrono::Utc::now());
                    for alert in crate::engine_watch::plan_alerts(&silent, &mut engine_alerts, std::time::Instant::now()) {
                        let Some(manager) = manager_exists(&state, &alert.project).await else { continue };
                        if let Err(e) = state.chat_manager.notify_event(&manager, &crate::engine_watch::message(&alert)).await {
                            tracing::debug!("Could not wake manager {manager}: {e}");
                        }
                    }
                }
            }

            // 2c. Patrol: a manager that is only ever woken by events or by the operator sits idle while
            //     things stall. Look at each project now and then, wake its manager with whatever is stuck
            //     or loose, and, when nothing is running and it has standing goals, point it at them.
            if patrol_checked.elapsed() >= Duration::from_secs(60) {
                patrol_checked = std::time::Instant::now();
                patrol(&state, &mut patrol_state).await;
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
                    correct_refusals(&state, &project, &mut corrections).await;
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
    fn a_mission_that_was_already_waiting_when_the_daemon_started_wakes_its_manager_once() {
        use MissionState::*;
        assert_eq!(plan_wakeup(true, None, Paused), Some(Wakeup::Catchup));
        assert_eq!(plan_wakeup(true, None, AwaitingApproval), Some(Wakeup::Catchup));
        // Finished or running missions are not announced on startup: that would be noise.
        for state in [Running, Planning, Validating, Review, Completed, Cancelled] {
            assert_eq!(plan_wakeup(true, None, state), None, "{state:?}");
        }
    }

    #[test]
    fn a_state_change_wakes_the_manager_and_no_change_does_not() {
        use MissionState::*;
        assert_eq!(plan_wakeup(false, Some(Running), Paused), Some(Wakeup::Changed));
        assert_eq!(plan_wakeup(false, None, Review), Some(Wakeup::Changed), "a new mission");
        // Still waiting is the patrol's to repeat, in one message with everything else that is stuck.
        assert_eq!(plan_wakeup(false, Some(Paused), Paused), None);
        assert_eq!(plan_wakeup(false, Some(Running), Running), None);
    }

    #[test]
    fn the_wake_message_says_why_and_tells_the_manager_to_unstick_a_pause() {
        let paused = wakeup_message("00007", "Add SSO", MissionState::Paused, Some("No agent output for 10 minutes"), Wakeup::Changed);
        assert!(paused.contains("Reason: No agent output for 10 minutes."), "{paused}");
        assert!(paused.contains("Unstick it yourself"), "{paused}");
        let catchup = wakeup_message("00007", "Add SSO", MissionState::AwaitingApproval, None, Wakeup::Catchup);
        assert!(catchup.contains("daemon restarted"), "{catchup}");
        assert!(!catchup.contains("Unstick"), "approval is not a pause: {catchup}");
    }

    #[test]
    fn refused_commands_are_read_off_the_turns_result_event() {
        let stream = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"on it"}]}}"#,
            r#"{"type":"result","subtype":"success","permission_denials":[{"tool_name":"Bash","tool_input":{"command":"tendril mission list --json | jq ."}}]}"#,
        ]
        .join("\n");
        let denials = refusals_in(&stream);
        assert_eq!(denials.len(), 1);
        assert_eq!(denials[0].tool_name, "Bash");
        let text = refusal_message(&denials);
        assert!(text.contains("tendril mission list"), "{text}");
        assert!(text.contains("Do not ask the operator to allow your shell"), "{text}");
        assert!(refusals_in(r#"{"type":"result","subtype":"success"}"#).is_empty());
        assert!(refusals_in("").is_empty());
    }

    #[test]
    fn a_manager_is_corrected_a_couple_of_times_then_left_to_it() {
        let mut history = HashMap::new();
        let t = std::time::Instant::now();
        assert!(may_correct(&mut history, "A", t));
        assert!(may_correct(&mut history, "A", t));
        assert!(!may_correct(&mut history, "A", t), "no loop on a command that can never work");
        assert!(may_correct(&mut history, "B", t), "another project is unaffected");
        assert!(may_correct(&mut history, "A", t + CORRECTION_WINDOW + Duration::from_secs(1)), "a fresh window");
    }

    #[test]
    fn only_the_states_a_manager_must_act_on_wake_it() {
        assert!(is_manager_business(MissionState::AwaitingApproval));
        assert!(is_manager_business(MissionState::Paused));
        assert!(is_manager_business(MissionState::Review));
        assert!(!is_manager_business(MissionState::Running));
        assert!(!is_manager_business(MissionState::Validating));
        assert!(!is_manager_business(MissionState::Planning));
        assert!(!is_manager_business(MissionState::Cancelled), "nothing to do about it, so no turn spent on it");
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
