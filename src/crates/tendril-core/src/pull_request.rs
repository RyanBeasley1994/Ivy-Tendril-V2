//! Creating a plan's pull request without an agent.
//!
//! Everything `CreatePr` does is mechanical - push, find-or-create the PR, comment, merge, record -
//! except resolving merge conflicts. So a `CreatePr` job runs [`run_native_create_pr`] first, and only
//! hands over to the agent (the same job, with a `Handoff` firmware header saying why) when it meets
//! something that needs judgement: a conflicting PR, a worktree that is gone, a branch in an odd
//! state. The common case costs seconds and no tokens, and the merge decision is an `if` rather than an
//! instruction an agent has to obey.
//!
//! GitHub is reached through [`GitHub`], so the flow is tested against a local bare remote with a fake
//! in place of `gh`; [`GhCli`] is the real one.

use crate::config::{expand_config_path, TendrilSettings};
use crate::error::{Result, TendrilError};
use crate::git::issues::parse_github_remote_url;
use crate::git::service::run_git;
use crate::git::worktree::{enumerate_worktree_directories, remove_worktree, resolve_repo_root};
use crate::jobs::firmware_values::find_project;
use crate::jobs::manager::apply_plan_state;
use crate::models::{canonical_pr_url, CreatePrArgs, PlanStatus};
use crate::plans::guards::PlanCompletionGuard;
use crate::plans::reader::read_plan_yaml;
use crate::plans::revisions::get_revision;
use crate::plans::writer::write_plan_yaml;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How a native attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativePrOutcome {
    /// PRs are recorded and the plan's state is settled. The job is done.
    Done { summary: String },
    /// Something needs judgement; the agent takes over the same job, told why.
    NeedsAgent { reason: String },
    /// A hard stop the agent could not fix either (a repo outside the project, wireframe code).
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRef {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct NewPr<'a> {
    pub repo: &'a str,
    pub base: &'a str,
    pub head: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub draft: bool,
    pub reviewers: &'a [String],
}

/// The GitHub operations the flow needs, each already retried on transient failures.
pub trait GitHub: Send + Sync {
    /// `owner/repo` for a remote URL; `None` for a remote that is not on GitHub.
    fn repo_slug(&self, remote_url: &str) -> Option<String> {
        parse_github_remote_url(remote_url).map(|(owner, name)| format!("{owner}/{name}"))
    }
    fn default_branch(&self, repo: &str) -> Result<String>;
    fn branch_visible(&self, repo: &str, branch: &str) -> bool;
    fn open_pr_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PrRef>>;
    fn create_pr(&self, pr: &NewPr<'_>) -> Result<PrRef>;
    fn comment(&self, repo: &str, number: u64, body: &str) -> Result<()>;
    /// `MERGEABLE`, `CONFLICTING` or `UNKNOWN`.
    fn mergeable(&self, repo: &str, number: u64) -> Result<String>;
    fn merge(&self, repo: &str, number: u64, delete_branch: bool) -> Result<()>;
}

/// Errors worth another try: the network, GitHub's 5xx, and rate limits. Auth, permissions,
/// validation and not-found fail at once.
fn is_transient(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    // A hook that failed is never the network, whatever it printed: a test run's "timed out" must not
    // make the push (and so the whole test suite) run three more times.
    if s.contains("hook") || s.contains("husky") {
        return false;
    }
    [
        "could not resolve host",
        "connection reset",
        "connection timed out",
        "could not connect",
        "failed to connect",
        "kex_exchange_identification",
        "early eof",
        "rpc failed",
        "tls",
        "http 5",
        "502",
        "503",
        "504",
        "429",
        "rate limit",
        "bad gateway",
        "operation timed out",
        "gateway timeout",
    ]
    .iter()
    .any(|needle| s.contains(needle))
}

/// Runs `f` up to four times, backing off 2s, 4s, 8s between transient failures.
fn with_retry<T>(mut f: impl FnMut() -> std::result::Result<T, String>) -> std::result::Result<T, String> {
    let mut delay = Duration::from_secs(2);
    let mut last = String::new();
    for attempt in 0..4 {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if attempt < 3 && is_transient(&e) => {
                last = e;
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// `gh` itself.
pub struct GhCli;

impl GhCli {
    fn run(&self, args: &[&str]) -> std::result::Result<String, String> {
        with_retry(|| {
            let out = Command::new("gh")
                .args(args)
                .output()
                .map_err(|e| format!("could not run gh: {e}"))?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        })
    }
}

fn gh_err(action: &str, e: String) -> TendrilError {
    TendrilError::Git(format!("gh {action} failed: {e}"))
}

impl GitHub for GhCli {
    fn default_branch(&self, repo: &str) -> Result<String> {
        self.run(&["repo", "view", repo, "--json", "defaultBranchRef", "-q", ".defaultBranchRef.name"])
            .map_err(|e| gh_err("repo view", e))
    }

    fn branch_visible(&self, repo: &str, branch: &str) -> bool {
        let path = format!("repos/{repo}/branches/{branch}");
        self.run(&["api", &path, "--silent"]).is_ok()
    }

    fn open_pr_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PrRef>> {
        let out = self
            .run(&["pr", "list", "--repo", repo, "--head", branch, "--state", "open", "--json", "number,url", "-q", ".[0]"])
            .map_err(|e| gh_err("pr list", e))?;
        if out.is_empty() || out == "null" {
            return Ok(None);
        }
        let v: serde_json::Value = serde_json::from_str(&out)?;
        Ok(v.get("number").and_then(|n| n.as_u64()).zip(v.get("url").and_then(|u| u.as_str())).map(
            |(number, url)| PrRef { number, url: url.to_string() },
        ))
    }

    fn create_pr(&self, pr: &NewPr<'_>) -> Result<PrRef> {
        // A body file, never an inline argument: bodies are long and full of quotes.
        // A unique name per call: concurrent CreatePr jobs share the temp directory, and a fixed name
        // once swapped PR bodies between plans (#1551).
        let body_file = std::env::temp_dir().join(format!("tendril-pr-body-{}.md", uuid::Uuid::new_v4().simple()));
        std::fs::write(&body_file, pr.body)?;
        let body_path = body_file.to_string_lossy().to_string();
        let mut args: Vec<String> = [
            "pr", "create", "--repo", pr.repo, "--base", pr.base, "--head", pr.head, "--title", pr.title,
            "--body-file", &body_path,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if pr.draft {
            args.push("--draft".into());
        }
        for reviewer in pr.reviewers.iter().map(|r| r.trim()).filter(|r| !r.is_empty()) {
            args.push("--reviewer".into());
            args.push(reviewer.to_string());
        }
        let with_assignee: Vec<&str> = args.iter().map(String::as_str).chain(["--assignee", "@me"]).collect();
        // Self-assignment is a nicety: an account that cannot assign itself still gets its PR.
        let created = match self.run(&with_assignee) {
            Ok(url) => Ok(url),
            Err(e) if e.to_ascii_lowercase().contains("assign") => {
                let plain: Vec<&str> = args.iter().map(String::as_str).collect();
                self.run(&plain).map_err(|e| gh_err("pr create", e))
            }
            Err(e) => Err(gh_err("pr create", e)),
        };
        let _ = std::fs::remove_file(&body_file);
        let url = created?;
        let url = url.lines().last().unwrap_or_default().trim().to_string();
        let number = url.rsplit('/').next().and_then(|n| n.parse().ok()).unwrap_or(0);
        Ok(PrRef { number, url })
    }

    fn comment(&self, repo: &str, number: u64, body: &str) -> Result<()> {
        let n = number.to_string();
        self.run(&["pr", "comment", &n, "--repo", repo, "--body", body])
            .map(|_| ())
            .map_err(|e| gh_err("pr comment", e))
    }

    fn mergeable(&self, repo: &str, number: u64) -> Result<String> {
        let n = number.to_string();
        self.run(&["pr", "view", &n, "--repo", repo, "--json", "mergeable", "-q", ".mergeable"])
            .map_err(|e| gh_err("pr view", e))
    }

    fn merge(&self, repo: &str, number: u64, delete_branch: bool) -> Result<()> {
        let n = number.to_string();
        let mut args = vec!["pr", "merge", &n, "--repo", repo, "--merge", "--admin"];
        if delete_branch {
            args.push("--delete-branch");
        }
        match self.run(&args) {
            Ok(_) => Ok(()),
            // A repo that only allows squash merges.
            Err(e) if e.contains("Merge commits are not allowed") => {
                let squash: Vec<&str> = args.iter().map(|a| if *a == "--merge" { "--squash" } else { a }).collect();
                self.run(&squash).map(|_| ()).map_err(|e| gh_err("pr merge", e))
            }
            Err(e) => Err(gh_err("pr merge", e)),
        }
    }
}

/// Where the flow reports what it is doing: the job's log and status line.
pub trait Progress {
    fn step(&mut self, message: &str);
}

impl<F: FnMut(&str)> Progress for F {
    fn step(&mut self, message: &str) {
        self(message)
    }
}

fn git_out(dir: &Path, args: &[&str]) -> std::result::Result<String, String> {
    match run_git(args, dir) {
        Ok((0, out, _)) => Ok(out.trim().to_string()),
        Ok((_, _, err)) => Err(err.trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

thread_local! {
    /// Where the running push records its process id, so cancelling the job can kill the push and
    /// every hook it started (a pre-push test run can take many minutes). Set by
    /// [`run_native_create_pr`] for the duration of one run on its thread.
    static PUSH_PID: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicU32>>> =
        const { std::cell::RefCell::new(None) };
}

/// `git push` for a plan branch. With `skip_hooks`, the repo's pre-push hook is bypassed both ways a
/// hook can run: git's own (`--no-verify`) and husky's (`HUSKY=0`, which also covers a husky set up to
/// ignore `--no-verify`).
fn git_push(dir: &Path, branch: &str, force: bool, skip_hooks: bool) -> std::result::Result<String, String> {
    let mut args = vec!["push"];
    if force {
        args.push("-f");
    }
    if skip_hooks {
        args.push("--no-verify");
    }
    args.extend(["-u", "origin", branch]);
    let mut cmd = Command::new("git");
    cmd.args(&args)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if skip_hooks {
        cmd.env("HUSKY", "0");
    }
    // Its own process group, so cancelling (which signals the group) takes the hook and everything it
    // started - a turbo test run - down with the push, instead of leaving them running for minutes.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    let pid_slot = PUSH_PID.with(|slot| slot.borrow().clone());
    if let Some(slot) = &pid_slot {
        slot.store(child.id(), std::sync::atomic::Ordering::SeqCst);
    }
    let out = child.wait_with_output();
    if let Some(slot) = &pid_slot {
        slot.store(0, std::sync::atomic::Ordering::SeqCst);
    }
    let out = out.map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// One repo's part of the plan: a worktree with commits to ship.
struct RepoPush {
    repo_root: PathBuf,
    worktree: PathBuf,
    slug: String,
    branch: String,
    base: String,
}

/// The PR body: the issue it fixes, the executor's summary (else the revision's Problem and
/// Solution), the commits, and the footer the agent writes.
fn build_body(plan_folder: &Path, source_url: Option<&str>, commits: &str) -> String {
    let mut body = String::new();
    if let Some(n) = source_url.and_then(|u| {
        let re = regex::Regex::new(r"github\.com/[^/]+/[^/]+/issues/(\d+)").ok()?;
        re.captures(u).map(|c| c[1].to_string())
    }) {
        body.push_str(&format!("Fixes #{n}\n\n"));
    }
    let summary = std::fs::read_to_string(plan_folder.join("Artifacts").join("summary.md"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            let revision = get_revision(plan_folder, None).unwrap_or_default();
            ["## Problem", "## Solution"]
                .iter()
                .filter_map(|heading| section(&revision, heading))
                .collect::<Vec<_>>()
                .join("\n\n")
        });
    body.push_str(summary.trim());
    body.push_str("\n\n---\n");
    body.push_str(commits.trim());
    body.push_str("\n\n---\nCreated using [Ivy Tendril](https://ivy.app).");
    body
}

/// A `## Heading` section of a markdown document, heading included, up to the next `## `.
fn section(doc: &str, heading: &str) -> Option<String> {
    let start = doc.find(heading)?;
    let rest = &doc[start..];
    let end = rest[heading.len()..].find("\n## ").map(|i| i + heading.len()).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

/// The installed CreatePr promptware ships an artifact upload tool: only the agent knows how to drive
/// it, so a plan with artifacts to upload goes to the agent.
fn has_artifact_tool(tendril_home: &Path) -> bool {
    std::fs::read_dir(tendril_home.join("Promptwares").join("CreatePr").join("Tools"))
        .map(|entries| entries.flatten().any(|e| !e.file_name().to_string_lossy().starts_with('.')))
        .unwrap_or(false)
}

fn has_artifact_media(plan_folder: &Path) -> bool {
    std::fs::read_dir(plan_folder.join("Artifacts"))
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                [".png", ".jpg", ".jpeg", ".gif", ".webp", ".mp4", ".webm", ".mov"]
                    .iter()
                    .any(|ext| name.ends_with(ext))
            })
        })
        .unwrap_or(false)
}

/// Tunables, so tests do not sleep for real.
#[derive(Debug, Clone)]
pub struct NativePrTiming {
    pub visibility_polls: u32,
    pub mergeable_polls: u32,
    pub poll_interval: Duration,
}

impl Default for NativePrTiming {
    fn default() -> Self {
        Self {
            visibility_polls: 5,
            mergeable_polls: 6,
            poll_interval: Duration::from_secs(3),
        }
    }
}

/// Creates (or updates) the plan's pull requests. See the module docs for when it hands over.
pub fn run_native_create_pr(
    plan_folder: &Path,
    args: &CreatePrArgs,
    settings: &TendrilSettings,
    tendril_home: &Path,
    gh: &dyn GitHub,
    timing: &NativePrTiming,
    progress: &mut dyn Progress,
) -> NativePrOutcome {
    run_native_create_pr_with_pid(plan_folder, args, settings, tendril_home, gh, timing, progress, None)
}

/// [`run_native_create_pr`], recording each push's process id in `push_pid` while it runs - the job's
/// own pid slot, which is what cancellation kills.
#[allow(clippy::too_many_arguments)]
pub fn run_native_create_pr_with_pid(
    plan_folder: &Path,
    args: &CreatePrArgs,
    settings: &TendrilSettings,
    tendril_home: &Path,
    gh: &dyn GitHub,
    timing: &NativePrTiming,
    progress: &mut dyn Progress,
    push_pid: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
) -> NativePrOutcome {
    PUSH_PID.with(|slot| *slot.borrow_mut() = push_pid);
    let outcome = match run(plan_folder, args, settings, tendril_home, gh, timing, progress) {
        Ok(outcome) => outcome,
        Err(e) => NativePrOutcome::NeedsAgent { reason: e.to_string() },
    };
    PUSH_PID.with(|slot| *slot.borrow_mut() = None);
    outcome
}

#[allow(clippy::too_many_lines)]
fn run(
    plan_folder: &Path,
    args: &CreatePrArgs,
    settings: &TendrilSettings,
    tendril_home: &Path,
    gh: &dyn GitHub,
    timing: &NativePrTiming,
    progress: &mut dyn Progress,
) -> Result<NativePrOutcome> {
    let (plan, _) = read_plan_yaml(plan_folder)?;
    let plan_id: String = plan_folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .chars()
        .take(5)
        .collect();

    if PlanStatus::from_str_loose(&plan.state) == Some(PlanStatus::Completed) {
        return Ok(NativePrOutcome::Done {
            summary: format!("Plan {plan_id} is already Completed: {}", plan.prs.join(", ")),
        });
    }

    let worktrees = enumerate_worktree_directories(&plan_folder.join("Worktrees"));
    if worktrees.is_empty() {
        return Ok(NativePrOutcome::NeedsAgent {
            reason: "the plan has no worktree on disk, so its branch has to be recovered from its commits".into(),
        });
    }
    if args.include_artifacts && has_artifact_tool(tendril_home) && has_artifact_media(plan_folder) {
        return Ok(NativePrOutcome::NeedsAgent {
            reason: "the plan has screenshots or videos to upload with the artifact tool".into(),
        });
    }

    progress.step("Checking the changes for wireframe code...");
    let project = find_project(settings, &plan.project);
    let leaks = crate::wireframes::plan_guard::check_and_report(plan_folder, project);
    if !leaks.is_empty() {
        return Ok(NativePrOutcome::Failed {
            reason: crate::wireframes::leak_guard::describe(&leaks),
        });
    }

    let allowed: Vec<PathBuf> = project
        .map(|p| {
            p.repos
                .iter()
                .map(|r| canonical(&expand_config_path(&r.path, tendril_home)))
                .collect()
        })
        .unwrap_or_default();

    // Gather every repo first, so a gate failure in the second repo never leaves the first pushed.
    let mut pushes = Vec::new();
    for worktree in &worktrees {
        let Some(repo_root) = resolve_repo_root(worktree) else {
            return Ok(NativePrOutcome::NeedsAgent {
                reason: format!("{} is not an attached git worktree", worktree.display()),
            });
        };
        if !allowed.contains(&canonical(&repo_root)) {
            return Ok(NativePrOutcome::Failed {
                reason: format!(
                    "{} is not one of project '{}''s repos; refusing to push or open a PR (the plan was likely created in the wrong project)",
                    repo_root.display(),
                    plan.project
                ),
            });
        }
        let remote = git_out(worktree, &["remote", "get-url", "origin"]).unwrap_or_default();
        let Some(slug) = gh.repo_slug(&remote) else {
            return Ok(NativePrOutcome::Failed {
                reason: format!("{} has no GitHub origin ({remote})", repo_root.display()),
            });
        };
        let branch = git_out(worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
        if branch.is_empty() || branch == "HEAD" {
            return Ok(NativePrOutcome::NeedsAgent {
                reason: format!("the worktree at {} is not on a branch", worktree.display()),
            });
        }
        if !git_out(worktree, &["status", "--porcelain"]).unwrap_or_default().is_empty() {
            return Ok(NativePrOutcome::NeedsAgent {
                reason: format!("the worktree at {} has uncommitted changes", worktree.display()),
            });
        }
        let base = match args.base_branch.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => b.to_string(),
            None => project
                .and_then(|p| {
                    p.repos.iter().find(|r| {
                        canonical(&expand_config_path(&r.path, tendril_home)) == canonical(&repo_root)
                    })
                })
                .and_then(|r| r.base_branch.clone())
                .map_or_else(|| gh.default_branch(&slug), Ok)?,
        };
        pushes.push(RepoPush {
            repo_root,
            worktree: worktree.clone(),
            slug,
            branch,
            base,
        });
    }

    let mut opened: Vec<(RepoPush, PrRef)> = Vec::new();
    for push in pushes {
        let _ = with_retry(|| git_out(&push.worktree, &["fetch", "origin", &push.base]));
        let base_ref = format!("origin/{}", push.base);
        let range = format!("{base_ref}..HEAD");
        let ahead: u32 = git_out(&push.worktree, &["rev-list", "--count", &range])
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        if ahead == 0 {
            progress.step(&format!("{}: no commits ahead of {}, skipping", push.slug, push.base));
            continue;
        }

        progress.step(&format!("Pushing {} to {}...", push.branch, push.slug));
        let skip_hooks = settings.git.skip_push_hooks == Some(true);
        let pushed = with_retry(|| git_push(&push.worktree, &push.branch, false, skip_hooks));
        if let Err(e) = pushed {
            // A diverged remote branch is stale state from an earlier run of this plan: the branch is
            // private to it, so the local history wins.
            if e.contains("non-fast-forward") || e.contains("rejected") || e.contains("fetch first") {
                with_retry(|| git_push(&push.worktree, &push.branch, true, skip_hooks))
                    .map_err(|e| TendrilError::Git(format!("push failed: {e}")))?;
            } else {
                return Err(TendrilError::Git(format!("push failed: {e}")));
            }
        }

        // GitHub's API lags a push by a moment; creating the PR before it sees the branch fails.
        let mut visible = false;
        for round in 0..2 {
            for _ in 0..timing.visibility_polls {
                if gh.branch_visible(&push.slug, &push.branch) {
                    visible = true;
                    break;
                }
                std::thread::sleep(timing.poll_interval);
            }
            if visible || round == 1 {
                break;
            }
            let _ = with_retry(|| git_push(&push.worktree, &push.branch, false, skip_hooks));
        }
        if !visible {
            return Err(TendrilError::Git(format!(
                "GitHub does not see {} on {} after pushing it",
                push.branch, push.slug
            )));
        }

        let pr = match gh.open_pr_for_branch(&push.slug, &push.branch)? {
            Some(existing) => {
                progress.step(&format!("Updated existing PR #{}", existing.number));
                existing
            }
            None => {
                progress.step(&format!("Creating a pull request on {}...", push.slug));
                let commits = git_out(&push.worktree, &["log", "--reverse", "--format=- %h %s", &range]).unwrap_or_default();
                let body = build_body(plan_folder, plan.source_url.as_deref(), &commits);
                let title = format!("[{plan_id}] {}", plan.title);
                let reviewers = args.reviewers.clone().unwrap_or_default();
                gh.create_pr(&NewPr {
                    repo: &push.slug,
                    base: &push.base,
                    head: &push.branch,
                    title: &title,
                    body: &body,
                    draft: args.draft,
                    reviewers: &reviewers,
                })?
            }
        };
        if let Some(comment) = args.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            gh.comment(&push.slug, pr.number, comment)?;
        }
        opened.push((push, pr));
    }

    if opened.is_empty() {
        return Ok(NativePrOutcome::Failed {
            reason: "No repo has commits ahead of its base branch, so there is nothing to open a pull request for".into(),
        });
    }

    // Recorded as soon as they exist, so nothing below can leave a real PR invisible to Tendril.
    record_prs(plan_folder, opened.iter().map(|(_, pr)| pr.url.as_str()))?;

    if args.solve_merge_conflicts || args.merge {
        for (push, pr) in &opened {
            let mut state = String::from("UNKNOWN");
            for _ in 0..timing.mergeable_polls {
                state = gh.mergeable(&push.slug, pr.number).unwrap_or_else(|_| "UNKNOWN".into());
                if state != "UNKNOWN" {
                    break;
                }
                std::thread::sleep(timing.poll_interval);
            }
            if state == "CONFLICTING" {
                if args.solve_merge_conflicts {
                    return Ok(NativePrOutcome::NeedsAgent {
                        reason: format!(
                            "PR #{} ({}) conflicts with {}; it is pushed and recorded, only the conflict resolution{} is left",
                            pr.number,
                            pr.url,
                            push.base,
                            if args.merge { " and the merge" } else { "" }
                        ),
                    });
                }
                return Ok(NativePrOutcome::Failed {
                    reason: format!(
                        "PR #{} ({}) conflicts with {} and resolving conflicts is turned off; it is recorded but not merged",
                        pr.number, pr.url, push.base
                    ),
                });
            }
        }
    }

    if args.merge {
        for (push, pr) in &opened {
            progress.step(&format!("Merging PR #{}...", pr.number));
            gh.merge(&push.slug, pr.number, args.delete_branch)?;
            // Bring the operator's checkout up to date, but only when it is clean and on the base.
            let clean = git_out(&push.repo_root, &["status", "--porcelain"]).is_ok_and(|s| s.is_empty());
            let on_base = git_out(&push.repo_root, &["symbolic-ref", "--short", "HEAD"]).is_ok_and(|b| b == push.base);
            if clean && on_base {
                let _ = with_retry(|| git_out(&push.repo_root, &["pull", "origin", &push.base]));
            }
            let repo_name = push.worktree.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if let Err(e) = remove_worktree(plan_folder, repo_name, Some(&push.branch)) {
                progress.step(&format!("Could not remove the worktree {}: {e}", push.worktree.display()));
            }
        }
    }

    let (plan, _) = read_plan_yaml(plan_folder)?;
    let failed = PlanCompletionGuard::failed_verifications(&plan);
    let urls = opened.iter().map(|(_, pr)| pr.url.clone()).collect::<Vec<_>>().join(", ");
    if failed.is_empty() {
        apply_plan_state(plan_folder, PlanStatus::Completed);
        crate::jobs::manager::sync_plan_state_to_db(tendril_home, plan_folder);
        Ok(NativePrOutcome::Done {
            summary: format!("Recorded PR: {urls} — plan {plan_id} state: Completed"),
        })
    } else {
        Ok(NativePrOutcome::Done {
            summary: format!(
                "Recorded PR: {urls} — plan {plan_id} left in {} because {} failed",
                plan.state,
                failed.join(", ")
            ),
        })
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn record_prs<'a>(plan_folder: &Path, urls: impl Iterator<Item = &'a str>) -> Result<()> {
    let (mut plan, _) = read_plan_yaml(plan_folder)?;
    let mut changed = false;
    for url in urls {
        let key = canonical_pr_url(url).unwrap_or_else(|| url.to_string());
        let known = plan
            .prs
            .iter()
            .any(|p| canonical_pr_url(p).unwrap_or_else(|| p.clone()) == key);
        if !known {
            plan.prs.push(url.to_string());
            changed = true;
        }
    }
    if changed {
        plan.updated = chrono::Utc::now();
        write_plan_yaml(plan_folder, &plan)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_told_apart_from_real_ones() {
        assert!(is_transient("fatal: unable to access: Could not resolve host: github.com"));
        assert!(is_transient("HTTP 502: Bad Gateway"));
        assert!(!is_transient("HTTP 403: Resource not accessible by integration"));
        assert!(!is_transient("GraphQL: No commits between main and feature"));
        assert!(!is_transient("husky - pre-push script failed (code 1)\nTest timed out in 5000ms"));
        assert!(!is_transient("error: failed to push some refs\nTest timed out in 5000ms"));
        assert!(is_transient("ssh: connect to host github.com port 22: Operation timed out"));
    }

    #[test]
    fn sections_are_cut_at_the_next_heading() {
        let doc = "# T\n\n## Problem\n\nbroken\n\n## Solution\n\nfix it\n\n## Tests\n\nrun";
        assert_eq!(section(doc, "## Problem").unwrap(), "## Problem\n\nbroken");
        assert_eq!(section(doc, "## Solution").unwrap(), "## Solution\n\nfix it");
        assert!(section(doc, "## Wireframe").is_none());
    }
}
