//! The native CreatePr path against a real git remote (a local bare repo) and a fake `gh`: create,
//! update, merge, conflicts, the project gate, and the hand-over cases.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tendril_core::config::TendrilSettings;
use tendril_core::error::Result;
use tendril_core::git::service::run_git;
use tendril_core::models::{CreatePrArgs, PlanVerificationEntry, PlanYaml, VerificationStatus};
use tendril_core::plans::reader::read_plan_yaml;
use tendril_core::plans::writer::write_plan_yaml;
use tendril_core::pull_request::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let (code, out, err) = run_git(args, dir).unwrap();
    assert_eq!(code, 0, "git {:?}: {}", args, err);
    out.trim().to_string()
}

#[derive(Default)]
struct FakeGh {
    remote: PathBuf,
    existing: Option<PrRef>,
    mergeable: String,
    calls: Mutex<Vec<String>>,
}

impl FakeGh {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn log(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl GitHub for FakeGh {
    fn repo_slug(&self, _remote_url: &str) -> Option<String> {
        Some("acme/widgets".into())
    }
    fn default_branch(&self, _repo: &str) -> Result<String> {
        Ok("main".into())
    }
    fn branch_visible(&self, _repo: &str, branch: &str) -> bool {
        let r = format!("refs/heads/{branch}");
        run_git(&["rev-parse", "--verify", "--quiet", &r], &self.remote).is_ok_and(|(c, _, _)| c == 0)
    }
    fn open_pr_for_branch(&self, _repo: &str, _branch: &str) -> Result<Option<PrRef>> {
        Ok(self.existing.clone())
    }
    fn create_pr(&self, pr: &NewPr<'_>) -> Result<PrRef> {
        self.log(format!("create {} <- {} draft={} reviewers={:?} title={}", pr.base, pr.head, pr.draft, pr.reviewers, pr.title));
        self.log(format!("body {}", pr.body));
        Ok(PrRef { number: 7, url: "https://github.com/acme/widgets/pull/7".into() })
    }
    fn comment(&self, _repo: &str, number: u64, body: &str) -> Result<()> {
        self.log(format!("comment #{number} {body}"));
        Ok(())
    }
    fn mergeable(&self, _repo: &str, _number: u64) -> Result<String> {
        Ok(self.mergeable.clone())
    }
    fn merge(&self, _repo: &str, number: u64, delete_branch: bool) -> Result<()> {
        self.log(format!("merge #{number} delete={delete_branch}"));
        Ok(())
    }
}

struct World {
    _dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    remote: PathBuf,
    plan: PathBuf,
    settings: TendrilSettings,
}

fn world(project_repo_is_this_one: bool) -> World {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let remote = dir.path().join("remote.git");
    let repo = dir.path().join("repo");
    let plans = dir.path().join("plans");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(dir.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(&repo, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.email", "t@example.com"), ("user.name", "T"), ("commit.gpgsign", "false")] {
        git(&repo, &["config", k, v]);
    }
    git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "root"]);
    git(&repo, &["push", "-q", "-u", "origin", "main"]);

    let plan = plans.join("00001-AddLogin");
    std::fs::create_dir_all(plan.join("Revisions")).unwrap();
    std::fs::write(plan.join("Revisions/001.md"), "# Add login\n\n## Problem\n\nNo login.\n\n## Solution\n\nAdd it.\n\n## Tests\n\nx").unwrap();
    let yaml = PlanYaml {
        state: "Review".into(),
        project: "P".into(),
        title: "Add login".into(),
        repos: vec![repo.to_string_lossy().to_string()],
        source_url: Some("https://github.com/acme/widgets/issues/12".into()),
        ..Default::default()
    };
    write_plan_yaml(&plan, &yaml).unwrap();
    let wt = plan.join("Worktrees/repo");
    git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "tendril/00001-AddLogin"]);
    std::fs::write(wt.join("login.txt"), "login\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-qm", "Add login"]);

    let project_path = if project_repo_is_this_one { repo.clone() } else { dir.path().join("elsewhere") };
    let settings: TendrilSettings = serde_yaml::from_str(&format!(
        "projects:\n  - name: P\n    repos:\n      - path: {}\n",
        project_path.display()
    ))
    .unwrap();
    World { _dir: dir, home, repo, remote, plan, settings }
}

fn args() -> CreatePrArgs {
    serde_json::from_value(serde_json::json!({ "folderPath": "x", "merge": false })).unwrap()
}

fn run(w: &World, gh: &FakeGh, args: &CreatePrArgs) -> (NativePrOutcome, Vec<String>) {
    let mut steps = Vec::new();
    let timing = NativePrTiming { visibility_polls: 2, mergeable_polls: 2, poll_interval: Duration::from_millis(1) };
    let mut progress = |m: &str| steps.push(m.to_string());
    let outcome = run_native_create_pr(&w.plan, args, &w.settings, &w.home, gh, &timing, &mut progress);
    (outcome, steps)
}

#[test]
fn opens_a_pr_records_it_and_completes_the_plan() {
    let w = world(true);
    let gh = FakeGh { remote: w.remote.clone(), mergeable: "MERGEABLE".into(), ..Default::default() };
    let mut a = args();
    a.draft = true;
    a.reviewers = Some(vec!["alice".into(), " ".into()]);
    a.comment = Some("Please look".into());
    let (outcome, _) = run(&w, &gh, &a);
    assert!(matches!(&outcome, NativePrOutcome::Done { summary } if summary.contains("Completed")), "{outcome:?}");

    let calls = gh.calls();
    assert!(calls[0].starts_with("create main <- tendril/00001-AddLogin draft=true"));
    assert!(calls[0].contains("title=[00001] Add login"));
    let body = &calls[1];
    assert!(body.starts_with("body Fixes #12\n\n## Problem\n\nNo login."));
    assert!(body.contains("Add login") && body.contains("Created using [Ivy Tendril]"));
    assert!(calls.iter().any(|c| c == "comment #7 Please look"));
    assert!(!calls.iter().any(|c| c.starts_with("merge")), "merge is off");

    let (plan, _) = read_plan_yaml(&w.plan).unwrap();
    assert_eq!(plan.state, "Completed");
    assert_eq!(plan.prs, vec!["https://github.com/acme/widgets/pull/7".to_string()]);
    assert!(w.plan.join("Worktrees/repo/.git").exists(), "no merge, so the worktree is kept");
    assert!(gh.branch_visible("", "tendril/00001-AddLogin"), "the branch was pushed");
}

#[test]
fn merges_when_asked_and_cleans_up() {
    let w = world(true);
    let gh = FakeGh { remote: w.remote.clone(), mergeable: "MERGEABLE".into(), ..Default::default() };
    let mut a = args();
    a.merge = true;
    a.delete_branch = true;
    let (outcome, _) = run(&w, &gh, &a);
    assert!(matches!(outcome, NativePrOutcome::Done { .. }), "{outcome:?}");
    assert!(gh.calls().iter().any(|c| c == "merge #7 delete=true"));
    assert!(!w.plan.join("Worktrees/repo").exists(), "worktree removed after the merge");
}

#[test]
fn an_existing_pr_is_updated_not_duplicated() {
    let w = world(true);
    let gh = FakeGh {
        remote: w.remote.clone(),
        existing: Some(PrRef { number: 3, url: "https://github.com/acme/widgets/pull/3".into() }),
        mergeable: "MERGEABLE".into(),
        ..Default::default()
    };
    let (outcome, steps) = run(&w, &gh, &args());
    assert!(matches!(outcome, NativePrOutcome::Done { .. }));
    assert!(!gh.calls().iter().any(|c| c.starts_with("create")));
    assert!(steps.iter().any(|s| s.contains("Updated existing PR #3")));
    assert_eq!(read_plan_yaml(&w.plan).unwrap().0.prs, vec!["https://github.com/acme/widgets/pull/3".to_string()]);
}

#[test]
fn a_conflict_hands_over_with_the_pr_already_recorded() {
    let w = world(true);
    let gh = FakeGh { remote: w.remote.clone(), mergeable: "CONFLICTING".into(), ..Default::default() };
    let mut a = args();
    a.merge = true;
    let (outcome, _) = run(&w, &gh, &a);
    match outcome {
        NativePrOutcome::NeedsAgent { reason } => assert!(reason.contains("conflicts") && reason.contains("merge")),
        other => panic!("{other:?}"),
    }
    assert!(!gh.calls().iter().any(|c| c.starts_with("merge")));
    let (plan, _) = read_plan_yaml(&w.plan).unwrap();
    assert_eq!(plan.prs.len(), 1);
    assert_eq!(plan.state, "Review", "the agent settles the state after resolving");
}

#[test]
fn a_repo_outside_the_project_is_refused_before_anything_is_pushed() {
    let w = world(false);
    let gh = FakeGh { remote: w.remote.clone(), ..Default::default() };
    let (outcome, _) = run(&w, &gh, &args());
    assert!(matches!(&outcome, NativePrOutcome::Failed { reason } if reason.contains("not one of project")), "{outcome:?}");
    assert!(!gh.branch_visible("", "tendril/00001-AddLogin"));
    assert!(gh.calls().is_empty());
}

#[test]
fn odd_states_go_to_the_agent() {
    // No worktree on disk.
    let w = world(true);
    git(&w.repo, &["worktree", "remove", "--force", w.plan.join("Worktrees/repo").to_str().unwrap()]);
    let gh = FakeGh { remote: w.remote.clone(), ..Default::default() };
    assert!(matches!(run(&w, &gh, &args()).0, NativePrOutcome::NeedsAgent { .. }));

    // Uncommitted changes in the worktree.
    let w = world(true);
    std::fs::write(w.plan.join("Worktrees/repo/dirty.txt"), "x").unwrap();
    let gh = FakeGh { remote: w.remote.clone(), ..Default::default() };
    assert!(matches!(&run(&w, &gh, &args()).0, NativePrOutcome::NeedsAgent { reason } if reason.contains("uncommitted")));
}

#[test]
fn a_failed_verification_records_the_pr_but_does_not_complete() {
    let w = world(true);
    let (mut plan, _) = read_plan_yaml(&w.plan).unwrap();
    plan.verifications = vec![PlanVerificationEntry { name: "Build".into(), status: VerificationStatus::Fail }];
    plan.state = "Failed".into();
    write_plan_yaml(&w.plan, &plan).unwrap();
    let gh = FakeGh { remote: w.remote.clone(), mergeable: "MERGEABLE".into(), ..Default::default() };
    let (outcome, _) = run(&w, &gh, &args());
    assert!(matches!(&outcome, NativePrOutcome::Done { summary } if summary.contains("Build failed")), "{outcome:?}");
    let (plan, _) = read_plan_yaml(&w.plan).unwrap();
    assert_eq!(plan.state, "Failed");
    assert_eq!(plan.prs.len(), 1);
}

#[cfg(unix)]
#[test]
fn a_failing_pre_push_hook_blocks_the_push_unless_skipping_is_allowed() {
    use std::os::unix::fs::PermissionsExt;
    let install_hook = |w: &World| {
        let hook = w.repo.join(".git/hooks/pre-push");
        std::fs::write(&hook, "#!/bin/sh\n[ \"$HUSKY\" = \"0\" ] && exit 0\necho 'tests timed out' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    };

    let w = world(true);
    install_hook(&w);
    let gh = FakeGh { remote: w.remote.clone(), mergeable: "MERGEABLE".into(), ..Default::default() };
    match run(&w, &gh, &args()).0 {
        NativePrOutcome::NeedsAgent { reason } => assert!(reason.contains("push failed"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(!gh.branch_visible("", "tendril/00001-AddLogin"), "the hook stopped the push");

    let mut w = world(true);
    install_hook(&w);
    w.settings.git.skip_push_hooks = Some(true);
    let gh = FakeGh { remote: w.remote.clone(), mergeable: "MERGEABLE".into(), ..Default::default() };
    assert!(matches!(run(&w, &gh, &args()).0, NativePrOutcome::Done { .. }));
    assert!(gh.branch_visible("", "tendril/00001-AddLogin"));
}
