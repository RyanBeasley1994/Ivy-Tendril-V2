//! The mission driver end to end, against a real git repo and a fake job engine: plan, approve,
//! execute, judge (retry, accept, re-plan), validate, hand over, and the pause paths in between.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tendril_core::config::TendrilSettings;
use tendril_core::error::Result;
use tendril_core::git::service::run_git;
use tendril_core::git::worktree::derive_branch_name;
use tendril_core::missions::driver::{BoxFuture, MissionDriver, MissionJobs};
use tendril_core::missions::model::*;
use tendril_core::missions::service::{self, MilestoneInput, MilestoneScope, MissionPaths, NewMission};
use tendril_core::missions::store::{read_mission, update_mission};
use tendril_core::models::{JobArgs, JobItem, JobStatus, PlanStatus};
use tendril_core::plans::dependencies::check_dependencies_with;
use tendril_core::plans::reader::read_plan_yaml;

#[derive(Default)]
struct FakeJobs {
    next: Mutex<u32>,
    jobs: Mutex<HashMap<String, (JobArgs, JobStatus, f64)>>,
    order: Mutex<Vec<String>>,
    agents: Mutex<HashMap<String, Option<RoleAgent>>>,
}

impl FakeJobs {
    fn last(&self) -> (String, JobArgs) {
        let id = self.order.lock().unwrap().last().cloned().expect("a job was started");
        let args = self.jobs.lock().unwrap()[&id].0.clone();
        (id, args)
    }
    fn count(&self) -> usize {
        self.order.lock().unwrap().len()
    }
    fn finish(&self, id: &str, status: JobStatus, cost: f64) {
        let mut jobs = self.jobs.lock().unwrap();
        let entry = jobs.get_mut(id).unwrap();
        entry.1 = status;
        entry.2 = cost;
    }
}

impl MissionJobs for FakeJobs {
    fn start<'a>(&'a self, args: JobArgs, agent: Option<&'a RoleAgent>) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            let id = format!("job-{}", n);
            self.agents.lock().unwrap().insert(id.clone(), agent.cloned());
            self.jobs.lock().unwrap().insert(id.clone(), (args, JobStatus::Running, 0.0));
            self.order.lock().unwrap().push(id.clone());
            Ok(id)
        })
    }
    fn job<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<JobItem>>> {
        Box::pin(async move {
            Ok(self.jobs.lock().unwrap().get(id).map(|(args, status, cost)| {
                let mut job = JobItem::new(id.to_string(), args.job_type().to_string(), String::new(), "P".into());
                job.status = *status;
                job.cost = Some(*cost);
                job
            }))
        })
    }
    fn cancel<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            self.finish(id, JobStatus::Stopped, 0.0);
            Ok(true)
        })
    }
}

fn git(repo: &Path, args: &[&str]) -> String {
    let (code, out, err) = run_git(args, repo).unwrap();
    assert_eq!(code, 0, "git {:?}: {}", args, err);
    out.trim().to_string()
}

struct World {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    paths: MissionPaths,
    jobs: Arc<FakeJobs>,
    driver: MissionDriver,
    settings: TendrilSettings,
}

fn world() -> World {
    world_with("")
}

/// A world whose `config.yaml` carries `extra` as further top-level keys.
fn world_with(extra: &str) -> World {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let plans = dir.path().join("plans");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "root"]);

    let settings: TendrilSettings = serde_yaml::from_str(&format!(
        "projects:\n  - name: P\n    repos:\n      - path: {}\n        baseBranch: main\n    verifications:\n      - name: Build\n        required: true\n{}",
        repo.display(),
        extra
    ))
    .unwrap();
    let paths = MissionPaths::new(&home, &plans);
    let jobs = Arc::new(FakeJobs::default());
    let provider_settings = settings.clone();
    let driver = MissionDriver::new(paths.clone(), jobs.clone(), Arc::new(move || provider_settings.clone()));
    World { _dir: dir, repo, paths, jobs, driver, settings }
}

fn milestone(title: &str) -> MilestoneInput {
    MilestoneInput {
        title: title.into(),
        objective: format!("Deliver {}", title),
        spec: "## Problem\n\nx\n\n## Solution\n\ny\n\n## Tests\n\nz".into(),
        acceptance: vec![format!("{} works", title)],
    }
}

/// Simulates ExecutePlan: cuts the milestone branch from the mission branch and commits on it.
fn execute(world: &World, plan_folder: &Path, mission_branch: &str, message: &str) {
    let branch = derive_branch_name(plan_folder);
    if run_git(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}", branch)], &world.repo).unwrap().0 != 0 {
        git(&world.repo, &["branch", &branch, mission_branch]);
    }
    git(&world.repo, &["checkout", "-q", &branch]);
    git(&world.repo, &["commit", "-q", "--allow-empty", "-m", message]);
    git(&world.repo, &["checkout", "-q", "main"]);
}

fn plan_folder_of(args: &JobArgs) -> PathBuf {
    PathBuf::from(args.plan_folder().unwrap())
}

#[tokio::test]
async fn a_mission_runs_from_goal_to_review() {
    let w = world();
    let created = service::create(
        &w.paths,
        &w.settings,
        NewMission { title: "Add SSO".into(), goal: "Users sign in with SSO".into(), project: "P".into(), ..Default::default() },
    )
    .unwrap();
    let folder = PathBuf::from(&created.folder_path);
    let integration = w.paths.plans_dir.join(created.mission.integration_plan.clone().unwrap());
    let branch = created.mission.branch.clone().unwrap();

    // The integration plan is held by the gate while the mission runs.
    let (yaml, _) = read_plan_yaml(&integration).unwrap();
    assert_eq!(yaml.state, "Blocked");
    assert_eq!(yaml.level, "Epic");
    let never = |_: &str| -> Result<String> { panic!("no PRs to resolve") };
    assert!(!check_dependencies_with(&integration, &w.paths.plans_dir, &never).unwrap().ok);

    // Planning: the orchestrator runs and writes milestones.
    assert!(w.driver.reconcile(&folder).await.unwrap());
    let (plan_job, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Plan"));
    assert!(!w.driver.reconcile(&folder).await.unwrap(), "nothing to do while planning runs");
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Backend"), milestone("Frontend")], MilestoneScope::All).unwrap();
    w.jobs.finish(&plan_job, JobStatus::Completed, 0.5);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::AwaitingApproval);
    assert_eq!(m.milestones.len(), 2);
    assert!((m.cost - 0.5).abs() < 1e-9);
    assert_eq!(w.jobs.count(), 1, "nothing runs before approval");

    // Approval: M1's plan is created on the mission branch and executed.
    service::approve(&folder).unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    let (exec1, args) = w.jobs.last();
    assert!(matches!(args, JobArgs::ExecutePlan(_)));
    let m1_plan = plan_folder_of(&args);
    let (m1_yaml, _) = read_plan_yaml(&m1_plan).unwrap();
    let link = mission_link(&m1_yaml).unwrap();
    assert_eq!(link.role, MissionRole::Milestone);
    assert_eq!(link.base_branch.as_deref(), Some(branch.as_str()));
    assert_eq!(git(&w.repo, &["rev-parse", &branch]), git(&w.repo, &["rev-parse", "main"]));

    // Execution finishes → judge.
    execute(&w, &m1_plan, &branch, "m1 first try");
    w.jobs.finish(&exec1, JobStatus::Completed, 1.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge1, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Judge" && a.milestone.as_deref() == Some("M1")));

    // Judge asks for a retry.
    service::decide(&folder, DecisionAction::Retry, Some(&judge1), "Missing tests", Some("Add unit tests"), None).unwrap();
    w.jobs.finish(&judge1, JobStatus::Completed, 0.2);
    w.driver.reconcile(&folder).await.unwrap();
    let (retry1, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::RetryPlan(a) if a.change_request == "Add unit tests"));
    assert_eq!(read_mission(&folder).unwrap().milestone("M1").unwrap().attempts, 2);

    // Retry finishes, judge accepts: the mission branch fast-forwards.
    execute(&w, &m1_plan, &branch, "m1 tests");
    w.jobs.finish(&retry1, JobStatus::Completed, 1.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge2, _) = w.jobs.last();
    service::decide(&folder, DecisionAction::Accept, Some(&judge2), "Meets criteria", None, Some("Backend done")).unwrap();
    w.jobs.finish(&judge2, JobStatus::Completed, 0.2);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    let m1 = m.milestone("M1").unwrap();
    assert_eq!(m1.state, MilestoneState::Passed);
    assert_eq!(m1.commits.len(), 2);
    assert_eq!(git(&w.repo, &["log", "-1", "--format=%s", &branch]), "m1 tests");
    assert_eq!(read_plan_yaml(&m1_plan).unwrap().0.state, "Completed");

    // M2 is cut from the advanced mission branch; the judge re-plans it into a fix-up.
    let (exec2, args) = w.jobs.last();
    assert!(matches!(args, JobArgs::ExecutePlan(_)));
    let m2_plan = plan_folder_of(&args);
    execute(&w, &m2_plan, &branch, "m2 wrong approach");
    w.jobs.finish(&exec2, JobStatus::Failed, 1.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge3, _) = w.jobs.last();
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Frontend v2")], MilestoneScope::Open).unwrap();
    service::decide(&folder, DecisionAction::Replan, Some(&judge3), "Wrong approach", None, None).unwrap();
    w.jobs.finish(&judge3, JobStatus::Completed, 0.2);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.milestone("M2").unwrap().state, MilestoneState::Skipped);
    assert_eq!(m.replans, 1);
    assert_eq!(read_plan_yaml(&m2_plan).unwrap().0.state, "Skipped");

    // M3 runs and is accepted.
    let (exec3, args) = w.jobs.last();
    let m3_plan = plan_folder_of(&args);
    execute(&w, &m3_plan, &branch, "m3");
    w.jobs.finish(&exec3, JobStatus::Completed, 1.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge4, _) = w.jobs.last();
    service::decide(&folder, DecisionAction::Accept, Some(&judge4), "Good", None, None).unwrap();
    w.jobs.finish(&judge4, JobStatus::Completed, 0.2);
    w.driver.reconcile(&folder).await.unwrap();

    // Validation: the mission branch is checked out in the integration plan, and the final phase runs.
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Validating);
    let (final_job, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Final"));
    let checkout = integration.join("Worktrees").join("repo");
    assert_eq!(git(&checkout, &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    assert!(service::decide(&folder, DecisionAction::Retry, Some(&final_job), "x", Some("y"), None).is_err());
    service::decide(&folder, DecisionAction::Accept, Some(&final_job), "All green", None, Some("SSO shipped")).unwrap();
    w.jobs.finish(&final_job, JobStatus::Completed, 0.3);
    w.driver.reconcile(&folder).await.unwrap();

    // Hand-over: the integration plan is in Review with every accepted commit, and the gate let go.
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Review);
    assert_eq!(m.summary.as_deref(), Some("SSO shipped"));
    let (yaml, _) = read_plan_yaml(&integration).unwrap();
    assert_eq!(yaml.state, PlanStatus::Review.to_string());
    assert_eq!(yaml.commits.len(), 3);
    assert!(check_dependencies_with(&integration, &w.paths.plans_dir, &never).unwrap().ok);
    assert!((m.cost - 5.6).abs() < 1e-9, "cost {}", m.cost);

    // The PR is created → the mission completes.
    tendril_core::jobs::manager::apply_plan_state(&integration, PlanStatus::Completed);
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(read_mission(&folder).unwrap().state, MissionState::Completed);
}

async fn approved_mission(w: &World, max_attempts: Option<u32>, max_cost: Option<f64>) -> PathBuf {
    let created = service::create(
        &w.paths,
        &w.settings,
        NewMission { title: "T".into(), goal: "G".into(), project: "P".into(), max_attempts, max_cost, ..Default::default() },
    )
    .unwrap();
    let folder = PathBuf::from(&created.folder_path);
    w.driver.reconcile(&folder).await.unwrap();
    let (plan_job, _) = w.jobs.last();
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Only")], MilestoneScope::All).unwrap();
    w.jobs.finish(&plan_job, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    service::approve(&folder).unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    folder
}

#[tokio::test]
async fn retries_stop_at_the_attempt_budget_and_resume_grants_a_fresh_one() {
    let w = world();
    let folder = approved_mission(&w, Some(1), None).await;
    let (exec, _) = w.jobs.last();
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge, _) = w.jobs.last();
    service::decide(&folder, DecisionAction::Retry, Some(&judge), "no", Some("again"), None).unwrap();
    w.jobs.finish(&judge, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Paused);
    assert!(m.pause_reason.unwrap().contains("did not pass after 1 attempts"));

    let before = w.jobs.count();
    service::resume(&folder).unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    let (_, args) = w.jobs.last();
    assert_eq!(w.jobs.count(), before + 1);
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Judge"), "resume re-judges what is there");
}

#[tokio::test]
async fn a_judge_without_a_decision_pauses_the_mission() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (exec, _) = w.jobs.last();
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge, _) = w.jobs.last();
    w.jobs.finish(&judge, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Paused);
    assert_eq!(m.paused_from, Some(MissionState::Running));
}

#[tokio::test]
async fn the_cost_budget_pauses_before_the_next_step() {
    let w = world();
    let folder = approved_mission(&w, None, Some(1.0)).await;
    let (exec, _) = w.jobs.last();
    w.jobs.finish(&exec, JobStatus::Completed, 2.0);
    let before = w.jobs.count();
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(w.jobs.count(), before, "no judge is started once the budget is spent");
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Paused);
    assert!(m.pause_reason.unwrap().contains("budget"));

    // Raising the budget and resuming carries on where it stopped.
    service::set_budget(&folder, None, None, Some(10.0)).unwrap();
    service::resume(&folder).unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    let (_, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Judge"));
}

#[tokio::test]
async fn cancelling_stops_the_current_job_and_releases_the_integration_plan() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (exec, _) = w.jobs.last();
    w.driver.cancel(&folder).await.unwrap();
    assert_eq!(read_mission(&folder).unwrap().state, MissionState::Cancelled);
    let job = w.jobs.jobs.lock().unwrap()[&exec].1;
    assert_eq!(job, JobStatus::Stopped);
    let m = read_mission(&folder).unwrap();
    let integration = w.paths.plans_dir.join(m.integration_plan.unwrap());
    let never = |_: &str| -> Result<String> { panic!() };
    assert!(check_dependencies_with(&integration, &w.paths.plans_dir, &never).unwrap().ok);
    assert!(!w.driver.reconcile(&folder).await.unwrap());
}

#[tokio::test]
async fn a_cleared_job_row_restarts_the_step() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    update_mission(&folder, |m| {
        m.current_job.as_mut().unwrap().job_id = "gone".into();
        Ok(())
    })
    .unwrap();
    let before = w.jobs.count();
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(w.jobs.count(), before + 1);
}

#[tokio::test]
async fn each_role_runs_on_its_own_harness() {
    let w = world();
    let created = service::create(
        &w.paths,
        &w.settings,
        NewMission {
            title: "T".into(),
            goal: "G".into(),
            project: "P".into(),
            agents: MissionAgents {
                planner: RoleAgent::parse("ClaudeCode:claude-opus-5-5"),
                worker: RoleAgent::parse("codex:gpt-5.6-sol:high"),
                judge: RoleAgent::parse("gemini"),
                validator: None,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let folder = PathBuf::from(&created.folder_path);
    let agent_of = |id: &str| w.jobs.agents.lock().unwrap()[id].clone();

    w.driver.reconcile(&folder).await.unwrap();
    let (plan_job, _) = w.jobs.last();
    let planner = agent_of(&plan_job).unwrap();
    assert_eq!(planner.agent, "claude", "agent ids are normalised");
    assert_eq!(planner.model.as_deref(), Some("claude-opus-5-5"));

    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Only")], MilestoneScope::All).unwrap();
    w.jobs.finish(&plan_job, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    service::approve(&folder).unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    let (exec, _) = w.jobs.last();
    assert_eq!(agent_of(&exec).unwrap().effort.as_deref(), Some("high"));

    // Changing the judge mid-mission applies to the next judge job.
    service::set_agents(
        &folder,
        MissionAgents { judge: RoleAgent::parse("copilot"), worker: RoleAgent::parse("codex"), ..Default::default() },
    )
    .unwrap();
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge, _) = w.jobs.last();
    assert_eq!(agent_of(&judge).unwrap().agent, "copilot");
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.current_job.unwrap().agent.as_deref(), Some("copilot"));
    assert!(m.agents.planner.is_none(), "set_agents replaces every role");

    service::decide(&folder, DecisionAction::Accept, Some(&judge), "ok", None, None).unwrap();
    w.jobs.finish(&judge, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (final_job, _) = w.jobs.last();
    assert!(agent_of(&final_job).is_none(), "an unset role runs on the default agent");
}

#[tokio::test]
async fn branches_follow_the_configured_templates() {
    let w = world_with(
        "git:\n  branchPrefix: rb/\n  branchTemplate: \"{prefix}{mission}-{milestone}-{slug}\"\n  missionBranchTemplate: \"{prefix}mission-{slug}\"\n",
    );
    let created = service::create(
        &w.paths,
        &w.settings,
        NewMission { title: "Add SSO".into(), goal: "G".into(), project: "P".into(), ..Default::default() },
    )
    .unwrap();
    assert_eq!(created.mission.branch.as_deref(), Some("rb/mission-add-sso"));
    let folder = PathBuf::from(&created.folder_path);
    w.driver.reconcile(&folder).await.unwrap();
    let (plan_job, _) = w.jobs.last();
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Backend")], MilestoneScope::All).unwrap();
    w.jobs.finish(&plan_job, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    service::approve(&folder).unwrap();
    w.driver.reconcile(&folder).await.unwrap();

    let (exec, args) = w.jobs.last();
    let plan = plan_folder_of(&args);
    let branch = derive_branch_name(&plan);
    assert_eq!(branch, "rb/00001-m1-m1-backend", "milestone plans use the plan template");
    execute(&w, &plan, "rb/mission-add-sso", "m1");
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge, _) = w.jobs.last();
    service::decide(&folder, DecisionAction::Accept, Some(&judge), "ok", None, None).unwrap();
    w.jobs.finish(&judge, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(git(&w.repo, &["log", "-1", "--format=%s", "rb/mission-add-sso"]), "m1");
}
