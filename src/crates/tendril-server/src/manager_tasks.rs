//! Small jobs a manager hands straight to one agent, and the loose ends it has to tie off.
//!
//! A plan job costs a planning run, an execution run under a long promptware, and the project's
//! verifications; a mission costs several of each. For a rename, a config tweak, a one-file fix or a
//! conflict that is most of the bill, and all of it is spent against the same rate limit the manager
//! itself runs on. A **task** is the light way: `tendril manager task --project P "<what to do>"` starts
//! one agent, on the project's worker engine, with nothing but the instruction, in a fresh worktree on
//! its own branch (or in a directory the manager names). When it stops, the manager is woken with what
//! it said and what it left behind.
//!
//! A task is an ordinary chat session, so it persists, streams, shows up in the app, falls back to
//! another agent on a rate limit, and can be sent back for more with its conversation intact.
//!
//! Nothing here merges or ships: landing the branch, pushing it, opening the pull request and removing
//! the worktree stay the manager's to see through. [`loose_ends`] is how the patrol holds it to that,
//! for tasks and for missions alike.

use crate::patrol::Finding;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tendril_core::chat::execution::ChatTurnOptions;
use tendril_core::chat::manager_brief::manager_session_id;
use tendril_core::git::service::run_git;
use tendril_core::missions::model::{MissionFile, MissionState};
use tokio::sync::Mutex;

/// Tasks running at once in one project. They skip the job queue, so this is the only thing between a
/// keen manager and a dozen agents drawing on one rate limit.
pub const MAX_RUNNING: usize = 2;
/// How much of the worker's last message the manager is shown.
const REPORT_CHARS: usize = 1500;
/// A finished task whose worktree is still there after this long is a loose end.
const UNCLEANED_AFTER_MINUTES: i64 = 30;
/// A task still running after this long is probably stuck.
const STUCK_AFTER_MINUTES: i64 = 45;
/// A worker that has written nothing for this long is hung, and is stopped. Chat turns have no
/// watchdog of their own (a person is normally watching them), and nobody is watching a task. Longer
/// than a job's ten minutes, because a task's worker runs its builds and tests inside the same turn.
pub const SILENT_AFTER: chrono::Duration = chrono::Duration::minutes(20);
/// However busy it looks, a "small job" still going after this long is stopped.
pub const LONGEST: chrono::Duration = chrono::Duration::minutes(120);
/// Completed missions older than this are no longer checked for an unshipped branch.
const MISSION_LOOKBACK_DAYS: i64 = 14;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project: String,
    pub title: String,
    /// The chat session the worker runs in.
    pub session_id: String,
    /// The repository the work belongs to.
    pub repo: String,
    /// Where the worker runs.
    pub dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// `HEAD` of `dir` when the task started, so what it added can be counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Whether `dir` is a worktree made for this task (and so removed with it), or somewhere that
    /// already existed and is left alone.
    pub own_worktree: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Why its worker was stopped, when it did not stop by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

impl Task {
    pub fn running(&self) -> bool {
        self.finished_at.is_none()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskFile {
    tasks: Vec<Task>,
}

static FILE_LOCK: Mutex<()> = Mutex::const_new(());

fn path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("manager-tasks.json")
}

fn read(tendril_home: &Path) -> TaskFile {
    std::fs::read_to_string(path(tendril_home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write(tendril_home: &Path, file: &TaskFile) {
    let _ = std::fs::write(path(tendril_home), serde_json::to_vec_pretty(file).unwrap_or_default());
}

pub fn list(tendril_home: &Path, project: &str) -> Vec<Task> {
    read(tendril_home).tasks.into_iter().filter(|t| t.project.eq_ignore_ascii_case(project)).collect()
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 32 {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// What a new task branch is cut from: the branch named, or the remote's main branch as it is on the
/// remote right now. Fetched first, so a task never starts from a stale copy of main; the local checkout
/// is only the fallback for a repository with no remote.
fn start_point(repo: &Path, from: Option<&str>) -> String {
    let _ = run_git(&["fetch", "origin", "--prune"], repo);
    if let Some(from) = from.map(str::trim).filter(|f| !f.is_empty()) {
        let remote = format!("origin/{from}");
        // A branch that exists here is used as it is here: it may hold work not pushed yet.
        return if git(repo, &["rev-parse", "--verify", "--quiet", from]).is_some() { from.to_string() } else { remote };
    }
    let main = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "origin/main".to_string());
    if git(repo, &["rev-parse", "--verify", "--quiet", &main]).is_some() { main } else { "HEAD".to_string() }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    match run_git(args, dir) {
        Ok((0, out, _)) => Some(out.trim().to_string()),
        _ => None,
    }
}

/// What the worker is told around the manager's instruction.
fn worker_prompt(project: &str, dir: &Path, branch: Option<&str>, own_worktree: bool, instruction: &str) -> String {
    let place = match (branch, own_worktree) {
        (Some(b), true) => format!("`{}`, a git worktree made for this task on its own branch `{b}`", dir.display()),
        (Some(b), false) => format!("`{}`, on branch `{b}`", dir.display()),
        (None, _) => format!("`{}`", dir.display()),
    };
    format!(
        "You are a worker on project \"{project}\", handed one small task by the project's manager. Nobody is watching this \
session and nobody will answer a question, so decide what you need to and get it done.\n\n\
You are working in {place}. Stay in it: do not change files outside it, do not switch branches, and do not push, merge \
or open a pull request unless the task says to.\n\n\
# The task\n{instruction}\n\n\
# Finishing\n\
Check your work the way the task asks, or the quickest honest way if it does not say (build it, run the tests it touches). \
Commit what you changed with a clear message. Then end with a short report, a few lines at most: what you changed, how you \
checked it and what the check showed, and anything you did not do or are not sure of. Say plainly if you could not do it."
    )
}

/// Starts a task and returns it.
pub async fn start(
    state: &AppState,
    project: &str,
    repo: &Path,
    title: &str,
    instruction: &str,
    dir: Option<&str>,
    from: Option<&str>,
) -> Result<Task, String> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err("A task needs an instruction".into());
    }
    let running = list(&state.tendril_home, project).iter().filter(|t| t.running()).count();
    if running >= MAX_RUNNING {
        return Err(format!(
            "{running} tasks are already running in this project, which is the limit. Wait for one to finish (you are woken when it does), or fold this into the next one."
        ));
    }
    let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect();
    let title = if title.trim().is_empty() { instruction.chars().take(60).collect::<String>() } else { title.trim().to_string() };

    let (dir, branch, own_worktree) = match dir.map(str::trim).filter(|d| !d.is_empty()) {
        Some(existing) => {
            let dir = PathBuf::from(existing);
            if !dir.is_dir() {
                return Err(format!("{existing} is not a directory"));
            }
            let branch = git(&dir, &["branch", "--show-current"]).filter(|b| !b.is_empty());
            (dir, branch, false)
        }
        None => {
            let dir = tendril_core::config::get_project_root_dir(&state.tendril_home, project).join("Tasks").join(&id);
            let branch = format!("tendril/task-{id}-{}", slug(&title)).trim_end_matches('-').to_string();
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let (repo_owned, dir_owned, branch_owned) = (repo.to_path_buf(), dir.clone(), branch.clone());
            let from = from.map(str::to_string);
            tokio::task::spawn_blocking(move || {
                let start = start_point(&repo_owned, from.as_deref());
                // `--no-track`: the new branch must not take the remote main as its upstream, or a bare
                // `git push` from it is aimed at main.
                match run_git(&["worktree", "add", "--no-track", "-b", &branch_owned, &dir_owned.to_string_lossy(), &start], &repo_owned) {
                    Ok((0, _, _)) => Ok(()),
                    Ok((_, _, err)) => Err(format!("Could not make a worktree for the task: {}", err.trim())),
                    Err(e) => Err(e.to_string()),
                }
            })
            .await
            .map_err(|e| e.to_string())??;
            (dir, Some(branch), true)
        }
    };
    let base = git(&dir, &["rev-parse", "HEAD"]);

    let engine = tendril_core::agents::project_engine::ProjectEngine::load_effective(&state.tendril_home, project);
    let worker = engine.role("worker").cloned();
    let session = state
        .chat_manager
        .create_session(
            Some(format!("Task · {title}")),
            worker.as_ref().map(|w| w.agent.clone()),
            worker.as_ref().and_then(|w| w.model.clone()),
            worker.as_ref().and_then(|w| w.effort.clone()),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;

    let task = Task {
        id,
        project: project.to_string(),
        title,
        session_id: session.id.clone(),
        repo: repo.to_string_lossy().to_string(),
        dir: dir.to_string_lossy().to_string(),
        branch,
        base,
        own_worktree,
        created_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        stopped: None,
    };
    let prompt = worker_prompt(project, &dir, task.branch.as_deref(), own_worktree, instruction);
    if let Err(e) = run_turn(state, &task, &prompt).await {
        // Nothing ran, so nothing can be lost: take the worktree and branch made a moment ago back out.
        if own_worktree {
            let _ = run_git(&["worktree", "remove", "--force", &task.dir], repo);
            if let Some(branch) = &task.branch {
                let _ = run_git(&["branch", "-D", branch], repo);
            }
        }
        return Err(e);
    }
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(&state.tendril_home);
    file.tasks.push(task.clone());
    write(&state.tendril_home, &file);
    Ok(task)
}

async fn run_turn(state: &AppState, task: &Task, prompt: &str) -> Result<(), String> {
    state
        .chat_manager
        .start_session_turn(
            &task.session_id,
            prompt,
            ChatTurnOptions { working_directory: Some(PathBuf::from(&task.dir)), ..Default::default() },
        )
        .await
        .map_err(|e| e.to_string())
}

/// Sends a task back to its worker with more to do. The worker keeps its conversation and its checkout.
pub async fn send_back(state: &AppState, project: &str, id: &str, instruction: &str) -> Result<Task, String> {
    let task = list(&state.tendril_home, project)
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("No task {id} in project {project}"))?;
    if task.running() {
        return Err(format!("Task {id} is still running; you are woken when it finishes"));
    }
    if !Path::new(&task.dir).is_dir() {
        return Err(format!("Task {id}'s checkout is gone; start a new task instead"));
    }
    let prompt = format!(
        "The manager looked at your work and sent it back:\n\n{}\n\nSame place, same rules. Fix it, commit, and end with a short report.",
        instruction.trim()
    );
    run_turn(state, &task, &prompt).await?;
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(&state.tendril_home);
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(t) = file.tasks.iter_mut().find(|t| t.id == id) {
        t.finished_at = None;
        t.stopped = None;
        // The clocks that decide whether it is stuck start again with the new round.
        t.created_at = now.clone();
    }
    write(&state.tendril_home, &file);
    Ok(Task { finished_at: None, stopped: None, created_at: now, ..task })
}

/// What a finished task left in its checkout.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Leftovers {
    pub commits: usize,
    pub uncommitted: usize,
}

fn leftovers(task: &Task) -> Leftovers {
    let dir = Path::new(&task.dir);
    let commits = task
        .base
        .as_deref()
        .and_then(|base| git(dir, &["rev-list", "--count", &format!("{base}..HEAD")]))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let uncommitted = git(dir, &["status", "--porcelain"]).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
    Leftovers { commits, uncommitted }
}

/// The message a manager is woken with when a task's worker stops.
pub fn finished_message(task: &Task, report: &str, left: &Leftovers) -> String {
    let report = report.trim();
    let report = if report.is_empty() {
        "(it said nothing: it was cut off, or the daemon restarted under it)".to_string()
    } else if report.chars().count() > REPORT_CHARS {
        format!("{}…", report.chars().take(REPORT_CHARS).collect::<String>())
    } else {
        report.to_string()
    };
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let place = match &task.branch {
        Some(b) => format!("on branch `{b}` in `{}`", task.dir),
        None => format!("in `{}`", task.dir),
    };
    let warn = if left.uncommitted > 0 { " Uncommitted work is not finished work." } else { "" };
    let stopped = match &task.stopped {
        Some(why) => format!(" It did not finish: it was stopped ({why}). What it left may be half done."),
        None => String::new(),
    };
    let clean = if task.own_worktree {
        format!(
            " If it is right, push the branch and open its pull request as your standing orders say (or leave it for the pull request the rest of the request goes into); remove its worktree with `tendril manager task-clean --project \"{}\" {}` once it is pushed.",
            task.project, task.id
        )
    } else {
        " If it is right, carry on with whatever it was holding up.".to_string()
    };
    format!(
        "Task {id} \"{title}\" has stopped.{stopped} Its worker's report:\n\n{report}\n\n\
It left {commits} and {files} {place}.{warn} A report is a claim: check what it changed (`git -C {dir} log --stat {base}..HEAD`) against what you asked for.{clean} \
If it is not, send it back with what is wrong: `tendril manager task --project \"{project}\" --continue {id} \"<what to fix>\"`.",
        id = task.id,
        title = task.title,
        commits = plural(left.commits, "commit"),
        files = plural(left.uncommitted, "uncommitted file"),
        dir = task.dir,
        base = task.base.as_deref().map(|b| b.chars().take(10).collect::<String>()).unwrap_or_else(|| "HEAD~1".into()),
        project = task.project,
    )
}

/// Why a running task should be stopped now, if it should: its worker has written nothing for too
/// long, or it has simply gone on too long to be the small job it was handed out as.
pub fn overdue(
    started: chrono::DateTime<chrono::Utc>,
    last_output: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    if now - last_output.max(started) >= SILENT_AFTER {
        Some(format!("its worker wrote nothing for {} minutes", SILENT_AFTER.num_minutes()))
    } else if now - started >= LONGEST {
        Some(format!("it had been running for {} minutes", LONGEST.num_minutes()))
    } else {
        None
    }
}

async fn mark_stopped(state: &AppState, id: &str, why: &str) {
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(&state.tendril_home);
    if let Some(t) = file.tasks.iter_mut().find(|t| t.id == id && t.stopped.is_none()) {
        t.stopped = Some(why.to_string());
    }
    write(&state.tendril_home, &file);
}

/// Stops a running task's worker. The manager is woken with what it left, as for any task that stops.
pub async fn stop(state: &AppState, project: &str, id: &str) -> Result<Task, String> {
    let task = list(&state.tendril_home, project)
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("No task {id} in project {project}"))?;
    if !task.running() {
        return Err(format!("Task {id} has already stopped"));
    }
    mark_stopped(state, id, "the manager stopped it").await;
    state.chat_manager.cancel_session(&task.session_id).await;
    Ok(task)
}

/// Wakes the manager of every task whose worker has stopped, and marks the task finished. A worker that
/// has hung is stopped first, so it is reported on the next pass instead of running for ever.
pub async fn report_finished(state: &AppState) {
    let running: Vec<Task> = read(&state.tendril_home).tasks.into_iter().filter(Task::running).collect();
    for mut task in running {
        if state.chat_manager.is_generating(&task.session_id).await {
            let now = chrono::Utc::now();
            let started = chrono::DateTime::parse_from_rfc3339(&task.created_at).map(|d| d.with_timezone(&chrono::Utc)).unwrap_or(now);
            let last_output = state.chat_manager.get_session(&task.session_id).await.map(|s| s.updated_at).unwrap_or(started);
            if let Some(why) = overdue(started, last_output, now) {
                tracing::warn!("Stopping task {} of {}: {why}", task.id, task.project);
                mark_stopped(state, &task.id, &why).await;
                state.chat_manager.cancel_session(&task.session_id).await;
            }
            continue;
        }
        // Read again: the reason it was stopped is written after the list above was taken.
        if let Some(fresh) = read(&state.tendril_home).tasks.into_iter().find(|t| t.id == task.id) {
            task.stopped = fresh.stopped;
        }
        let report = state
            .chat_manager
            .get_session(&task.session_id)
            .await
            .ok()
            .and_then(|s| s.messages.iter().rev().find(|m| m.role == "assistant").map(|m| m.content.clone()))
            .unwrap_or_default();
        let for_git = task.clone();
        let left = tokio::task::spawn_blocking(move || leftovers(&for_git)).await.unwrap_or_default();
        {
            let _guard = FILE_LOCK.lock().await;
            let mut file = read(&state.tendril_home);
            if let Some(t) = file.tasks.iter_mut().find(|t| t.id == task.id) {
                t.finished_at = Some(chrono::Utc::now().to_rfc3339());
            }
            write(&state.tendril_home, &file);
        }
        let manager = manager_session_id(&task.project);
        if let Err(e) = state.chat_manager.notify_event(&manager, &finished_message(&task, &report, &left)).await {
            tracing::debug!("Could not tell manager {manager} that task {} finished: {e}", task.id);
        }
    }
}

/// Whether a branch's commits are safe somewhere else, which under a pull-request workflow means on a
/// remote: its own pushed branch, or main once its pull request merged. A branch with nothing of its own
/// (it never moved from where it was cut) is safe too.
fn branch_is_safe(repo: &Path, branch: &str) -> bool {
    git(repo, &["branch", "-r", "--contains", branch]).is_some_and(|out| !out.is_empty())
}

/// Removes a finished task's worktree, and its branch when the commits are safe elsewhere. Returns what
/// it did, in words.
pub async fn clean(state: &AppState, project: &str, id: &str, force: bool) -> Result<String, String> {
    let task = list(&state.tendril_home, project)
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("No task {id} in project {project}"))?;
    if task.running() {
        return Err(format!("Task {id} is still running"));
    }
    let for_git = task.clone();
    let outcome = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let task = for_git;
        if !task.own_worktree {
            return Ok(format!("Task {} forgotten. It ran in {}, which was not made for it and is left as it is.", task.id, task.dir));
        }
        let (repo, dir) = (Path::new(&task.repo), Path::new(&task.dir));
        let mut said = Vec::new();
        if dir.exists() {
            let dirty = leftovers(&task).uncommitted;
            if dirty > 0 && !force {
                return Err(format!(
                    "Its worktree has {dirty} uncommitted file(s). Send the task back to commit them, or pass --force to throw them away."
                ));
            }
            let dir_text = dir.to_string_lossy();
            let mut args = vec!["worktree", "remove"];
            if force {
                args.push("--force");
            }
            args.push(&dir_text);
            match run_git(&args, repo) {
                Ok((0, _, _)) => said.push("worktree removed".to_string()),
                Ok((_, _, err)) => return Err(format!("Could not remove the worktree: {}", err.trim())),
                Err(e) => return Err(e.to_string()),
            }
        }
        if let Some(branch) = &task.branch {
            if git(repo, &["rev-parse", "--verify", "--quiet", branch]).is_some() {
                if force || branch_is_safe(repo, branch) {
                    let _ = run_git(&["branch", "-D", branch], repo);
                    said.push(format!("branch {branch} deleted"));
                } else {
                    said.push(format!(
                        "branch {branch} KEPT: its commits are on no remote, so deleting it would lose them. Push it first; if its pull request was squash-merged and the remote branch deleted, the work is in main and --force is right"
                    ));
                }
            }
        }
        Ok(format!("Task {}: {}.", task.id, if said.is_empty() { "nothing left to clean".to_string() } else { said.join("; ") }))
    })
    .await
    .map_err(|e| e.to_string())??;
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(&state.tendril_home);
    file.tasks.retain(|t| t.id != id);
    write(&state.tendril_home, &file);
    Ok(outcome)
}

fn minutes_since(rfc3339: &str, now: chrono::DateTime<chrono::Utc>) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|d| (now - d.with_timezone(&chrono::Utc)).num_minutes())
        .unwrap_or(0)
}

/// Tasks that are stuck, or finished and not tidied away.
pub fn task_findings(tasks: &[Task], now: chrono::DateTime<chrono::Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    for t in tasks {
        match &t.finished_at {
            None => {
                let age = minutes_since(&t.created_at, now);
                if age >= STUCK_AFTER_MINUTES {
                    out.push(Finding {
                        key: format!("task {} running", t.id),
                        text: format!(
                            "task {} \"{}\" has been running for {age} min, far too long for a small task: and it is stopped by the daemon if its worker goes silent for 20: if it should not wait that long, stop it (`tendril manager task-stop --project \"{}\" {}`) and hand out a smaller piece",
                            t.id, t.title, t.project, t.id
                        ),
                    });
                }
            }
            Some(at) if t.own_worktree => {
                let age = minutes_since(at, now);
                if age >= UNCLEANED_AFTER_MINUTES {
                    out.push(Finding {
                        key: format!("task {} uncleaned", t.id),
                        text: format!(
                            "task {} \"{}\" finished {age} min ago and its worktree and branch{} are still there: check it, then push it and open its pull request or send it back, then `tendril manager task-clean --project \"{}\" {}`",
                            t.id,
                            t.title,
                            t.branch.as_deref().map(|b| format!(" `{b}`")).unwrap_or_default(),
                            t.project,
                            t.id
                        ),
                    });
                }
            }
            Some(_) => {}
        }
    }
    out
}

/// Where a finished mission's branch stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchFate {
    /// The branch is gone: nothing left to do.
    Gone,
    /// It has commits that are on no remote.
    Unpushed,
    /// Pushed, but no pull request was ever opened from it.
    NoPullRequest,
    /// Its pull request is open: watching that is `watch-pr`'s business.
    Open(u64),
    /// Its pull request merged or closed and the branch is still here.
    Finished(u64, String),
}

/// The loose end a finished mission's branch is, if it is one.
pub fn mission_finding(id: &str, title: &str, branch: &str, integration_plan: Option<&str>, fate: &BranchFate) -> Option<Finding> {
    let text = match fate {
        BranchFate::Gone | BranchFate::Open(_) => return None,
        BranchFate::Unpushed => format!(
            "mission {id} \"{title}\" is completed but its branch `{branch}` has commits that were never pushed: push it and open a pull request, then watch its checks"
        ),
        BranchFate::NoPullRequest => format!(
            "mission {id} \"{title}\" is completed and `{branch}` is pushed, but it has no pull request: open one and watch its checks"
        ),
        BranchFate::Finished(pr, state) => format!(
            "mission {id} \"{title}\": pull request {pr} is {} but branch `{branch}` and its worktree are still here: remove them{}",
            state.to_lowercase(),
            integration_plan.map(|p| format!(" (`tendril plan cleanup {p}`, then delete the local branch)")).unwrap_or_default()
        ),
    };
    let kind = match fate {
        BranchFate::Unpushed => "unpushed",
        BranchFate::NoPullRequest => "no-pr",
        _ => "uncleaned",
    };
    Some(Finding { key: format!("mission {id} {kind}"), text })
}

async fn branch_fate(repo: &Path, branch: &str) -> BranchFate {
    let (repo_owned, branch_owned) = (repo.to_path_buf(), branch.to_string());
    let local = tokio::task::spawn_blocking(move || {
        git(&repo_owned, &["rev-parse", "--verify", "--quiet", &branch_owned])?;
        Some(git(&repo_owned, &["branch", "-r", "--contains", &branch_owned]).is_some_and(|o| !o.is_empty()))
    })
    .await
    .unwrap_or(None);
    let Some(pushed) = local else { return BranchFate::Gone };
    let mut cmd = tokio::process::Command::new("gh");
    cmd.current_dir(repo).args(["pr", "list", "--head", branch, "--state", "all", "--limit", "1", "--json", "number,state"]);
    let prs: Vec<serde_json::Value> = match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => serde_json::from_slice(&out.stdout).unwrap_or_default(),
        // No answer from GitHub is not evidence of anything: say only what git itself showed.
        _ => return if pushed { BranchFate::Open(0) } else { BranchFate::Unpushed },
    };
    match prs.first() {
        Some(pr) => {
            let number = pr["number"].as_u64().unwrap_or(0);
            match pr["state"].as_str().unwrap_or("OPEN") {
                "OPEN" => BranchFate::Open(number),
                state => BranchFate::Finished(number, state.to_string()),
            }
        }
        None if pushed => BranchFate::NoPullRequest,
        None => BranchFate::Unpushed,
    }
}

/// What a project's manager has left untidy: tasks stuck or not cleaned up, and completed missions whose
/// branch was never pushed, never became a pull request, or outlived the one it became.
pub async fn loose_ends(
    state: &AppState,
    project: &str,
    repo: Option<&Path>,
    missions: &[MissionFile],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Finding> {
    let mut out = task_findings(&list(&state.tendril_home, project), now);
    let Some(repo) = repo else { return out };
    for f in missions {
        let m = &f.mission;
        if m.state != MissionState::Completed || now - m.updated > chrono::Duration::days(MISSION_LOOKBACK_DAYS) {
            continue;
        }
        let Some(branch) = m.branch.as_deref() else { continue };
        let fate = branch_fate(repo, branch).await;
        out.extend(mission_finding(&f.id, &m.title, branch, m.integration_plan.as_deref(), &fate));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, own_worktree: bool) -> Task {
        Task {
            id: id.into(),
            project: "Acme".into(),
            title: "Rename the flag".into(),
            session_id: "s".into(),
            repo: "/r".into(),
            dir: "/w".into(),
            branch: Some("tendril/task-1-rename-the-flag".into()),
            base: Some("0123456789abcdef".into()),
            own_worktree,
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            stopped: None,
        }
    }

    #[test]
    fn the_worker_is_told_where_it_is_and_how_to_finish() {
        let text = worker_prompt("Acme", Path::new("/w"), Some("b"), true, "Rename --foo to --bar");
        assert!(text.contains("Rename --foo to --bar"));
        assert!(text.contains("own branch `b`"));
        assert!(text.contains("do not push, merge"));
        assert!(text.contains("Commit what you changed"));
    }

    #[test]
    fn the_manager_is_told_what_was_left_and_to_check_it() {
        let text = finished_message(&task("ab12", true), "Renamed it. Tests pass.", &Leftovers { commits: 1, uncommitted: 2 });
        assert!(text.contains("Renamed it. Tests pass."), "{text}");
        assert!(text.contains("1 commit and 2 uncommitted files"), "{text}");
        assert!(text.contains("Uncommitted work is not finished work"), "{text}");
        assert!(text.contains("A report is a claim"), "{text}");
        assert!(text.contains("tendril manager task-clean --project \"Acme\" ab12"), "{text}");
        assert!(text.contains("--continue ab12"), "{text}");
        // A worker that was cut off says nothing, and the manager is told so rather than shown a blank.
        let cut = finished_message(&task("ab12", false), "  ", &Leftovers::default());
        assert!(cut.contains("it said nothing") && !cut.contains("task-clean"), "{cut}");
        // A long report is clipped.
        let long = finished_message(&task("ab12", true), &"x".repeat(5000), &Leftovers::default());
        assert!(long.len() < 3000);
    }

    #[test]
    fn a_silent_or_endless_worker_is_stopped_and_a_working_one_is_not() {
        let now = chrono::Utc::now();
        let ago = |m: i64| now - chrono::Duration::minutes(m);
        assert_eq!(overdue(ago(30), ago(2), now), None, "still writing");
        assert_eq!(overdue(ago(5), ago(5), now), None, "only just started");
        assert!(overdue(ago(40), ago(21), now).unwrap().contains("wrote nothing for 20 minutes"));
        // A session last touched before the task began (a task sent back) is timed from the new start.
        assert_eq!(overdue(ago(3), ago(300), now), None);
        assert!(overdue(ago(121), ago(1), now).unwrap().contains("running for 120 minutes"));
        // The manager is told it did not finish.
        let stopped = Task { stopped: Some("its worker wrote nothing for 20 minutes".into()), ..task("ab12", true) };
        let text = finished_message(&stopped, "", &Leftovers { commits: 0, uncommitted: 116 });
        assert!(text.contains("It did not finish: it was stopped (its worker wrote nothing for 20 minutes)"), "{text}");
        assert!(text.contains("116 uncommitted files"), "{text}");
    }

    #[test]
    fn a_stuck_task_and_an_uncleaned_one_are_loose_ends() {
        let now = chrono::Utc::now();
        let ago = |m: i64| (now - chrono::Duration::minutes(m)).to_rfc3339();
        let fresh = task("1", true);
        let stuck = Task { created_at: ago(60), ..task("2", true) };
        let done_recently = Task { finished_at: Some(ago(5)), ..task("3", true) };
        let done_long_ago = Task { finished_at: Some(ago(90)), ..task("4", true) };
        let borrowed_dir = Task { finished_at: Some(ago(90)), ..task("5", false) };
        let found = task_findings(&[fresh, stuck, done_recently, done_long_ago, borrowed_dir], now);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].key, "task 2 running");
        assert_eq!(found[1].key, "task 4 uncleaned");
        assert!(found[1].text.contains("task-clean"));
    }

    #[test]
    fn a_finished_missions_branch_is_a_loose_end_until_it_is_shipped_and_gone() {
        let f = |fate| mission_finding("00007", "Add SSO", "tendril/mission-7", Some("00031-AddSso"), &fate);
        assert!(f(BranchFate::Gone).is_none());
        assert!(f(BranchFate::Open(46)).is_none(), "an open PR is being watched, not nagged about");
        assert!(f(BranchFate::Unpushed).unwrap().text.contains("never pushed"));
        assert!(f(BranchFate::NoPullRequest).unwrap().text.contains("no pull request"));
        let merged = f(BranchFate::Finished(46, "MERGED".into())).unwrap();
        assert!(merged.text.contains("pull request 46 is merged") && merged.text.contains("tendril plan cleanup 00031-AddSso"), "{merged:?}");
        // The key says which loose end, so moving from one to the next is news.
        assert_ne!(f(BranchFate::Unpushed).unwrap().key, f(BranchFate::NoPullRequest).unwrap().key);
    }

    #[test]
    fn branch_names_are_short_and_plain() {
        assert_eq!(slug("Rename --foo to --bar!"), "rename-foo-to-bar");
        assert!(slug(&"word ".repeat(40)).len() <= 32);
        assert_eq!(slug("***"), "");
    }
}
