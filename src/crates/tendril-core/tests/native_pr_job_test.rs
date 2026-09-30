//! A CreatePr *job* takes the native path: a clean plan finishes without an agent ever being
//! launched, and a conflict hands the same job to the agent with the reason in its firmware.

mod common;

use common::HomeFixture;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tendril_core::agents::providers::AgentProcessSpec;
use tendril_core::config::TendrilSettings;
use tendril_core::error::Result;
use tendril_core::git::service::run_git;
use tendril_core::jobs::manager::JobManager;
use tendril_core::models::{JobArgs, JobStatus, PlanYaml};
use tendril_core::plans::reader::read_plan_yaml;
use tendril_core::pull_request::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let (code, out, err) = run_git(args, dir).unwrap();
    assert_eq!(code, 0, "git {:?}: {}", args, err);
    out.trim().to_string()
}

struct Gh {
    remote: PathBuf,
    mergeable: &'static str,
}

impl GitHub for Gh {
    fn repo_slug(&self, _: &str) -> Option<String> {
        Some("acme/widgets".into())
    }
    fn default_branch(&self, _: &str) -> Result<String> {
        Ok("main".into())
    }
    fn branch_visible(&self, _: &str, branch: &str) -> bool {
        let r = format!("refs/heads/{branch}");
        run_git(&["rev-parse", "--verify", "--quiet", &r], &self.remote).is_ok_and(|(c, _, _)| c == 0)
    }
    fn open_pr_for_branch(&self, _: &str, _: &str) -> Result<Option<PrRef>> {
        Ok(None)
    }
    fn create_pr(&self, _: &NewPr<'_>) -> Result<PrRef> {
        Ok(PrRef { number: 9, url: "https://github.com/acme/widgets/pull/9".into() })
    }
    fn comment(&self, _: &str, _: u64, _: &str) -> Result<()> {
        Ok(())
    }
    fn mergeable(&self, _: &str, _: u64) -> Result<String> {
        Ok(self.mergeable.into())
    }
    fn merge(&self, _: &str, _: u64, _: bool) -> Result<()> {
        Ok(())
    }
}

/// A home with a CreatePr promptware, a git remote, and a plan in Review with one committed worktree.
fn setup(home: &HomeFixture) -> (PathBuf, PathBuf, TendrilSettings) {
    home.write_promptware("CreatePr");
    let remote = home.path.join("remote.git");
    let repo = home.path.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&home.path, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(&repo, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.email", "t@example.com"), ("user.name", "T"), ("commit.gpgsign", "false")] {
        git(&repo, &["config", k, v]);
    }
    git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "root"]);
    git(&repo, &["push", "-q", "-u", "origin", "main"]);

    let plan = home.write_plan(
        "00001-Ship",
        &PlanYaml {
            state: "Review".into(),
            project: "P".into(),
            title: "Ship".into(),
            repos: vec![repo.to_string_lossy().to_string()],
            ..Default::default()
        },
    );
    let wt = plan.join("Worktrees/repo");
    git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "tendril/00001-Ship"]);
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "work"]);

    let settings: TendrilSettings = serde_yaml::from_str(&format!(
        "maxConcurrentJobs: 2\nprojects:\n  - name: P\n    repos:\n      - path: {}\n        baseBranch: main\n",
        repo.display()
    ))
    .unwrap();
    (plan, remote, settings)
}

async fn run_job(home: &HomeFixture, settings: TendrilSettings, gh: Gh, plan: &Path) -> (JobStatus, usize, String) {
    let launches = Arc::new(AtomicUsize::new(0));
    let counter = launches.clone();
    let workdir = home.path.clone();
    let manager = JobManager::new(home.path.clone(), settings)
        .with_plans_dir(Some(home.plans_dir()))
        .with_github(
            Arc::new(gh),
            NativePrTiming { visibility_polls: 2, mergeable_polls: 2, poll_interval: Duration::from_millis(1) },
        )
        .with_spec_builder(Arc::new(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            AgentProcessSpec {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "exit 0".into()],
                environment: Default::default(),
                working_directory: workdir.clone(),
                stdin_content: None,
                redirect_stdin: false,
                temp_files: vec![],
            }
        }))
        .share();
    let args: JobArgs = serde_json::from_value(serde_json::json!({
        "type": "CreatePr",
        "folderPath": plan.to_string_lossy(),
        "merge": false
    }))
    .unwrap();
    let id = manager.start_job(args).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let job = manager.get_job(&id).await.unwrap().unwrap();
        if !matches!(job.status, JobStatus::Pending | JobStatus::Queued | JobStatus::Running) {
            let prompt = std::fs::read_to_string(tendril_core::jobs::logger::get_prompt_path(&home.path, &id))
                .unwrap_or_default();
            return (job.status, launches.load(Ordering::SeqCst), prompt);
        }
        assert!(tokio::time::Instant::now() < deadline, "the job never finished");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_clean_plan_ships_without_an_agent() {
    let home = HomeFixture::new("native-pr-clean");
    let (plan, remote, settings) = setup(&home);
    let (status, launches, _) = run_job(&home, settings, Gh { remote, mergeable: "MERGEABLE" }, &plan).await;
    assert_eq!(status, JobStatus::Completed);
    assert_eq!(launches, 0, "no agent was launched");
    let (yaml, _) = read_plan_yaml(&plan).unwrap();
    assert_eq!(yaml.state, "Completed");
    assert_eq!(yaml.prs, vec!["https://github.com/acme/widgets/pull/9".to_string()]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_conflict_hands_the_job_to_the_agent() {
    let home = HomeFixture::new("native-pr-conflict");
    let (plan, remote, settings) = setup(&home);
    let (_, launches, prompt) = run_job(&home, settings, Gh { remote, mergeable: "CONFLICTING" }, &plan).await;
    assert_eq!(launches, 1, "the agent took over");
    assert!(prompt.contains("Handoff:") && prompt.contains("conflicts with main"), "{prompt}");
}

#[cfg(unix)]
#[tokio::test]
async fn turning_native_off_always_uses_the_agent() {
    let home = HomeFixture::new("native-pr-off");
    let (plan, remote, mut settings) = setup(&home);
    settings.git.native_pull_requests = Some(false);
    let (_, launches, prompt) = run_job(&home, settings, Gh { remote, mergeable: "MERGEABLE" }, &plan).await;
    assert_eq!(launches, 1);
    assert!(!prompt.contains("Handoff:"));
}
