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
use tendril_core::missions::service::{self, MilestoneInput, MilestoneScope, MissionPaths, NewMission, TaskInput};
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
    messages: Mutex<HashMap<String, String>>,
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
    /// Ends a job the way an agent CLI that hit its limit does: failed, with the CLI's message.
    fn fail_with(&self, id: &str, message: &str) {
        self.messages.lock().unwrap().insert(id.to_string(), message.to_string());
        self.finish(id, JobStatus::Failed, 0.0);
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
                job.status_message = self.messages.lock().unwrap().get(id).cloned();
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
        tasks: vec![
            TaskInput { title: format!("{} part one", title), done_when: "one is done".into() },
            TaskInput { title: format!("{} part two", title), done_when: "two is done".into() },
        ],
        provides: Vec::new(),
        consumes: Vec::new(),
    }
}

/// The mission's one shared checkout of the test repo, which every milestone works in.
fn shared_checkout(world: &World, plan_folder: &Path) -> PathBuf {
    let (plan, _) = read_plan_yaml(plan_folder).unwrap();
    let link = mission_link(&plan).unwrap();
    let mission = read_mission(Path::new(&link.folder)).unwrap();
    let integration = world.paths.plans_dir.join(mission.integration_plan.unwrap());
    integration.join("Worktrees").join("repo")
}

/// Simulates ExecutePlan: asks for its worktree the way the promptware does, and commits there.
fn execute(world: &World, plan_folder: &Path, mission_branch: &str, message: &str) {
    let creation = tendril_core::git::worktree::add_worktree(
        &world.repo,
        plan_folder,
        None,
        tendril_core::git::worktree::WorktreeMode::ReuseIfValid,
        None,
    )
    .unwrap();
    assert!(creation.shared, "a milestone plan gets the mission's shared checkout");
    assert_eq!(creation.path, shared_checkout(world, plan_folder));
    assert_eq!(creation.branch, mission_branch);
    git(&creation.path, &["commit", "-q", "--allow-empty", "-m", message]);
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
    assert!(matches!(&args, JobArgs::RetryPlan(a) if a.change_request.ends_with("Add unit tests") && a.change_request.contains("shared worktree")));
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
    // The dropped milestone's work is rolled back off the shared branch.
    assert_eq!(git(&w.repo, &["log", "-1", "--format=%s", &branch]), "m1 tests");
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
    let summary = std::fs::read_to_string(integration.join("Artifacts/summary.md")).unwrap();
    assert!(summary.starts_with("# Summary\n\nSSO shipped"), "{summary}");
    assert!(summary.contains("### M1 — Backend") && summary.contains("Backend done"));
    assert!(summary.contains("- [x] Frontend v2 works"));
    assert!(!summary.contains("M2 — Frontend\n"), "skipped milestones are left out");
    assert!(summary.contains("| Build | Pending |"));

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

#[tokio::test]
async fn a_rate_limited_execution_waits_then_continues_without_using_an_attempt() {
    let w = world();
    let folder = approved_mission(&w, Some(3), None).await;
    let (exec, args) = w.jobs.last();
    let plan = plan_folder_of(&args);
    execute(&w, &plan, &read_mission(&folder).unwrap().branch.unwrap(), "half done");
    service::set_task_done(&folder, "M1.1", true).unwrap();
    w.jobs.fail_with(&exec, "Claude AI usage limit reached, try again in 23 minutes");
    w.driver.reconcile(&folder).await.unwrap();

    let m = read_mission(&folder).unwrap();
    let wait = m.rate_limit.clone().expect("the mission waits out the limit");
    assert_eq!(wait.step, MissionStep::Execute);
    assert_eq!(m.state, MissionState::Running, "a limit does not pause the mission");
    assert_eq!(m.milestone("M1").unwrap().state, MilestoneState::Executing, "and does not go to the judge");
    assert_eq!(m.milestone("M1").unwrap().attempts, 0, "waiting is not an attempt");
    let count = w.jobs.count();
    assert!(!w.driver.reconcile(&folder).await.unwrap(), "nothing happens until the wait is over");
    assert_eq!(w.jobs.count(), count);

    // The wait passes: the milestone continues in place, told what is already done.
    update_mission(&folder, |m| {
        m.rate_limit.as_mut().unwrap().until = chrono::Utc::now() - chrono::Duration::seconds(1);
        Ok(())
    })
    .unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    let (resume, args) = w.jobs.last();
    assert_ne!(resume, exec);
    assert!(matches!(&args, JobArgs::RetryPlan(a) if a.change_request.contains("rate or usage limit") && a.change_request.contains("M1.1")));
    let m = read_mission(&folder).unwrap();
    assert!(m.rate_limit.is_none());
    assert_eq!(m.milestone("M1").unwrap().attempts, 1);
}

#[tokio::test]
async fn a_rate_limited_judge_is_rerun_after_the_wait() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (exec, _) = w.jobs.last();
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge, _) = w.jobs.last();
    w.jobs.fail_with(&judge, "HTTP 429 Too Many Requests");
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(read_mission(&folder).unwrap().state, MissionState::Running);
    update_mission(&folder, |m| {
        m.rate_limit.as_mut().unwrap().until = chrono::Utc::now() - chrono::Duration::seconds(1);
        Ok(())
    })
    .unwrap();
    w.driver.reconcile(&folder).await.unwrap();
    let (again, args) = w.jobs.last();
    assert_ne!(again, judge);
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Judge"));
}

#[tokio::test]
async fn a_real_failure_is_not_mistaken_for_a_rate_limit() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (exec, _) = w.jobs.last();
    w.jobs.fail_with(&exec, "cargo test failed: 3 tests failed");
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert!(m.rate_limit.is_none());
    assert_eq!(m.milestone("M1").unwrap().state, MilestoneState::Judging, "a failure still goes to the judge");
}

#[tokio::test]
async fn contracts_must_be_provided_before_they_are_consumed() {
    let w = world();
    let created = service::create(
        &w.paths,
        &w.settings,
        NewMission { title: "C".into(), goal: "G".into(), project: "P".into(), ..Default::default() },
    )
    .unwrap();
    let folder = PathBuf::from(&created.folder_path);
    let store = ContractItem {
        name: "SessionStore".into(),
        kind: "type".into(),
        signature: "struct SessionStore".into(),
        location: String::new(),
    };
    let mut producer = milestone("Store");
    producer.provides = vec![store.clone()];
    let mut consumer = milestone("Login");
    consumer.consumes = vec!["SessionStore".into()];

    // Consumed before it is provided: rejected, and nothing is written.
    let err = service::set_milestones(&folder, &w.paths.plans_dir, vec![consumer.clone(), producer.clone()], MilestoneScope::All)
        .unwrap_err()
        .to_string();
    assert!(err.contains("consumes 'SessionStore'"), "{err}");
    assert!(read_mission(&folder).unwrap().milestones.is_empty());

    // Provided twice: rejected.
    let err = service::set_milestones(&folder, &w.paths.plans_dir, vec![producer.clone(), producer.clone()], MilestoneScope::All)
        .unwrap_err()
        .to_string();
    assert!(err.contains("provided by both"), "{err}");

    // In order: accepted, with task ids assigned.
    service::set_milestones(&folder, &w.paths.plans_dir, vec![producer, consumer], MilestoneScope::All).unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.milestone("M2").unwrap().consumes, vec!["SessionStore".to_string()]);
    assert_eq!(m.milestone("M1").unwrap().tasks[1].id, "M1.2");
    let consumed = service::consumed_items(&m, m.milestone("M2").unwrap());
    assert_eq!(consumed.len(), 1);
    assert_eq!(consumed[0].0, "M1");
}

#[test]
fn a_milestone_needs_a_real_task_breakdown() {
    let one_task = r#"
milestones:
  - title: Big
    spec: body
    acceptance: [works]
    tasks:
      - title: do everything
"#;
    let err = service::parse_milestones(one_task).unwrap_err().to_string();
    assert!(err.contains("1 tasks"), "{err}");
}

#[tokio::test]
async fn tasks_are_ticked_off_and_shown_in_the_milestone_plan() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (_, args) = w.jobs.last();
    let plan = plan_folder_of(&args);
    let body = tendril_core::plans::revisions::get_revision(&plan, None).unwrap();
    assert!(body.contains("## Tasks") && body.contains("**M1.1**") && body.contains("tendril mission task"), "{body}");

    service::set_task_done(&folder, "M1.2", true).unwrap();
    let m = read_mission(&folder).unwrap();
    assert!(m.milestone("M1").unwrap().tasks[1].done);
    assert!(service::set_task_done(&folder, "M9.1", true).is_err());
}

#[tokio::test]
async fn the_final_phase_cannot_accept_until_every_verification_is_recorded() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (exec, _) = w.jobs.last();
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (judge, _) = w.jobs.last();
    service::decide(&folder, DecisionAction::Accept, Some(&judge), "ok", None, None).unwrap();
    w.jobs.finish(&judge, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (final_job, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Final"));

    // `Build` is still Pending on the integration plan: the accept is refused, with the fix.
    let err = service::decide_checked(&folder, &w.paths.plans_dir, DecisionAction::Accept, Some(&final_job), "all green", None, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Build") && err.contains("set-verification"), "{err}");

    // Recorded: the accept goes through.
    let integration = w.paths.plans_dir.join(read_mission(&folder).unwrap().integration_plan.unwrap());
    let (mut yaml, _) = read_plan_yaml(&integration).unwrap();
    for v in yaml.verifications.iter_mut() {
        v.status = tendril_core::models::VerificationStatus::Pass;
    }
    tendril_core::plans::writer::write_plan_yaml(&integration, &yaml).unwrap();
    service::decide_checked(&folder, &w.paths.plans_dir, DecisionAction::Accept, Some(&final_job), "all green", None, None).unwrap();
}

#[tokio::test]
async fn a_chat_on_a_mission_plan_is_briefed_and_runs_in_its_worktree() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (_, args) = w.jobs.last();
    let m1_plan = plan_folder_of(&args);
    execute(&w, &m1_plan, &read_mission(&folder).unwrap().branch.unwrap(), "m1 work");

    // The chat reads the Plans folder from config.yaml, as the daemon does.
    std::fs::write(
        w.paths.tendril_home.join("config.yaml"),
        format!("planFolder: {}\n", w.paths.plans_dir.display()),
    )
    .unwrap();
    let name = m1_plan.file_name().unwrap().to_string_lossy().to_string();
    let ctx = tendril_core::chat::execution::plan_context::plan_chat_context(&w.paths.tendril_home, &name)
        .expect("a plan chat gets context");

    let worktree = ctx.working_directory.expect("it runs in the plan's worktree");
    assert_eq!(
        std::fs::canonicalize(&worktree).unwrap(),
        std::fs::canonicalize(shared_checkout(&w, &m1_plan)).unwrap(),
        "a milestone's chat runs in the mission's shared checkout"
    );
    let b = &ctx.briefing;
    assert!(b.contains("# Plan Context") && b.contains("milestone M1 of the mission"), "{b}");
    assert!(b.contains("Goal:") && b.contains("- M1 [Executing]"), "{b}");
    assert!(b.contains("m1 work"), "recent commits are listed: {b}");
    assert!(b.contains("## The plan") && b.contains("## Tasks"), "{b}");
    assert!(b.contains("## Serving the app") && b.contains("http://localhost:<port>"), "the chat knows how to serve: {b}");
}

/// Runs the open milestone through execute and an accepting judge, then the final phase to Review.
async fn run_to_review(w: &World, folder: &Path) {
    let branch = read_mission(folder).unwrap().branch.unwrap();
    while read_mission(folder).unwrap().state == MissionState::Running {
        let (job, args) = w.jobs.last();
        match &args {
            JobArgs::ExecutePlan(_) | JobArgs::RetryPlan(_) => {
                execute(w, &plan_folder_of(&args), &branch, "work");
                w.jobs.finish(&job, JobStatus::Completed, 0.0);
            }
            JobArgs::OrchestrateMission(a) if a.phase == "Judge" => {
                service::decide(folder, DecisionAction::Accept, Some(&job), "ok", None, None).unwrap();
                w.jobs.finish(&job, JobStatus::Completed, 0.0);
            }
            other => panic!("unexpected job while running: {other:?}"),
        }
        w.driver.reconcile(folder).await.unwrap();
    }
    let (final_job, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Final"));
    service::decide(folder, DecisionAction::Accept, Some(&final_job), "green", None, None).unwrap();
    w.jobs.finish(&final_job, JobStatus::Completed, 0.0);
    w.driver.reconcile(folder).await.unwrap();
    assert_eq!(read_mission(folder).unwrap().state, MissionState::Review);
}

#[tokio::test]
async fn a_change_request_in_review_sends_the_mission_back_through_its_own_milestones() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    run_to_review(&w, &folder).await;
    let integration = w.paths.plans_dir.join(read_mission(&folder).unwrap().integration_plan.unwrap());
    assert_eq!(read_plan_yaml(&integration).unwrap().0.state, "Review");

    // Not while it is still running a job, not with no text.
    assert!(service::request_changes(&folder, &w.paths.plans_dir, "  ").is_err());

    let cr = service::request_changes(&folder, &w.paths.plans_dir, "Rename the button to Save").unwrap();
    assert_eq!(cr, "C1");
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Planning);
    assert_eq!(read_plan_yaml(&integration).unwrap().0.state, "Blocked", "the plan is held again");

    // Revise: the orchestrator plans fix-ups for the request, which run without a fresh approval.
    w.driver.reconcile(&folder).await.unwrap();
    let (revise, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Revise"));
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Rename button")], MilestoneScope::Pending).unwrap();
    w.jobs.finish(&revise, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Running, "paused: {:?}; log: {:#?}", m.pause_reason, m.log.iter().rev().take(5).collect::<Vec<_>>());
    assert_eq!(m.milestone("M1").unwrap().state, MilestoneState::Passed, "accepted work is kept");
    let c1 = &m.change_requests[0];
    assert_eq!(c1.state, ChangeRequestState::Planned);
    assert_eq!(c1.milestones, vec!["M2".to_string()]);

    // The fix-up runs, is judged, the mission is validated again and returns to Review.
    run_to_review(&w, &folder).await;
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.change_requests[0].state, ChangeRequestState::Done);
    assert_eq!(read_plan_yaml(&integration).unwrap().0.state, "Review");
}

#[tokio::test]
async fn changes_can_only_be_requested_once_the_mission_is_in_review() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let err = service::request_changes(&folder, &w.paths.plans_dir, "x").unwrap_err().to_string();
    assert!(err.contains("in Review"), "{err}");
}

#[tokio::test]
async fn a_message_before_approval_is_answered_now_and_the_plan_waits_again() {
    let w = world();
    let created = service::create(
        &w.paths,
        &w.settings,
        NewMission { title: "T".into(), goal: "G".into(), project: "P".into(), ..Default::default() },
    )
    .unwrap();
    let folder = PathBuf::from(&created.folder_path);
    w.driver.reconcile(&folder).await.unwrap();
    let (plan_job, _) = w.jobs.last();
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Backend")], MilestoneScope::All).unwrap();
    w.jobs.finish(&plan_job, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(read_mission(&folder).unwrap().state, MissionState::AwaitingApproval);

    let posted = service::post_message(&folder, &w.paths.plans_dir, "Split it into backend and frontend").unwrap();
    assert_eq!((posted.id.as_str(), posted.delivery.as_str()), ("Q1", "immediate"));
    w.driver.reconcile(&folder).await.unwrap();
    let (steer, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Steer"));

    // The orchestrator revises the plan and replies.
    service::set_milestones(&folder, &w.paths.plans_dir, vec![milestone("Backend"), milestone("Frontend")], MilestoneScope::All).unwrap();
    service::reply_to_message(&folder, "Q1", "Split into M1 backend and M2 frontend").unwrap();
    w.jobs.finish(&steer, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::AwaitingApproval, "still the operator's to approve");
    assert_eq!(m.milestones.len(), 2);
    assert_eq!(m.messages[0].reply.as_deref(), Some("Split into M1 backend and M2 frontend"));
    let count = w.jobs.count();
    assert!(!w.driver.reconcile(&folder).await.unwrap(), "an answered message starts nothing");
    assert_eq!(w.jobs.count(), count);
}

#[tokio::test]
async fn a_message_while_running_waits_for_the_next_decision_point() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (exec, args) = w.jobs.last();
    assert!(matches!(args, JobArgs::ExecutePlan(_)));
    let posted = service::post_message(&folder, &w.paths.plans_dir, "Use Postgres, not SQLite").unwrap();
    assert_eq!(posted.delivery, "next");
    let count = w.jobs.count();
    w.driver.reconcile(&folder).await.unwrap();
    assert_eq!(w.jobs.count(), count, "the running worker is not interrupted");
    assert!(read_mission(&folder).unwrap().messages[0].is_open());

    // The judge is the next decision point; it sees the message.
    w.jobs.finish(&exec, JobStatus::Completed, 0.0);
    w.driver.reconcile(&folder).await.unwrap();
    let (_, args) = w.jobs.last();
    assert!(matches!(&args, JobArgs::OrchestrateMission(a) if a.phase == "Judge"));
    assert!(read_mission(&folder).unwrap().open_messages().next().is_some());
}

#[tokio::test]
async fn a_message_in_review_becomes_a_change_request() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    run_to_review(&w, &folder).await;
    let posted = service::post_message(&folder, &w.paths.plans_dir, "The button should say Save").unwrap();
    assert_eq!(posted.delivery, "changeRequest");
    assert_eq!(posted.change_request.as_deref(), Some("C1"));
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Planning);
    assert_eq!(m.messages[0].became_change_request.as_deref(), Some("C1"));
}

#[tokio::test]
async fn a_quick_fix_in_review_keeps_the_mission_in_review_and_reaches_the_pull_request() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    run_to_review(&w, &folder).await;
    let integration = w.paths.plans_dir.join(read_mission(&folder).unwrap().integration_plan.unwrap());
    std::fs::create_dir_all(integration.join("Artifacts")).unwrap();
    std::fs::write(integration.join("Artifacts/summary.md"), "# Summary\n\nDone.").unwrap();

    // The Review chat is told it can make the change itself, and how to record it.
    std::fs::write(
        w.paths.tendril_home.join("config.yaml"),
        format!("planFolder: {}\n", w.paths.plans_dir.display()),
    )
    .unwrap();
    let name = integration.file_name().unwrap().to_string_lossy().to_string();
    let b = tendril_core::chat::execution::plan_context::plan_chat_context(&w.paths.tendril_home, &name)
        .unwrap()
        .briefing;
    assert!(b.contains("## Quick changes") && b.contains("tendril mission quick-fix 00001"), "{b}");
    assert!(!b.contains("Ask before committing"), "a plan in Review may be committed to: {b}");

    service::record_quick_fix(&folder, &w.paths.plans_dir, "Button says Save", &["abc1234".into()]).unwrap();
    let m = read_mission(&folder).unwrap();
    assert_eq!(m.state, MissionState::Review, "no trip back through planning");
    assert_eq!(m.quick_fixes[0].summary, "Button says Save");
    assert!(read_plan_yaml(&integration).unwrap().0.commits.contains(&"abc1234".to_string()));
    let summary = std::fs::read_to_string(integration.join("Artifacts/summary.md")).unwrap();
    assert!(summary.contains("## Quick fixes in review") && summary.contains("- Button says Save"), "{summary}");
}

#[tokio::test]
async fn quick_fixes_are_only_for_a_mission_in_review() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    assert!(service::record_quick_fix(&folder, &w.paths.plans_dir, "x", &[]).is_err());
}

#[tokio::test]
async fn a_milestone_that_committed_but_kept_no_records_still_delivered() {
    let w = world();
    let folder = approved_mission(&w, None, None).await;
    let (_, args) = w.jobs.last();
    let m1_plan = plan_folder_of(&args);
    execute(&w, &m1_plan, &read_mission(&folder).unwrap().branch.unwrap(), "m1: the whole milestone");
    let (yaml, _) = read_plan_yaml(&m1_plan).unwrap();
    assert!(yaml.commits.is_empty(), "the worker recorded nothing");
    assert!(yaml.verifications.iter().any(|v| v.status == tendril_core::models::VerificationStatus::Pending));

    let mut job = JobItem::new("job-x".into(), "ExecutePlan".into(), m1_plan.to_string_lossy().to_string(), "P".into());
    let outcome = tendril_core::jobs::deliverable::verify_deliverable(&w.paths.plans_dir, &mut job, &[]);
    assert_eq!(outcome, tendril_core::jobs::deliverable::Deliverable::Present, "the commit is on the branch");
    let (yaml, _) = read_plan_yaml(&m1_plan).unwrap();
    assert_eq!(yaml.commits.len(), 1, "read back from the shared branch");
    assert_eq!(
        tendril_core::plans::verification_gate::resolve_post_execution_state(&yaml, &m1_plan, None),
        PlanStatus::Review,
        "the judge, not this gate, verifies a milestone"
    );

    // A check the worker recorded as failed still fails the run.
    let mut failed = yaml.clone();
    failed.verifications[0].status = tendril_core::models::VerificationStatus::Fail;
    tendril_core::plans::writer::write_plan_yaml(&m1_plan, &failed).unwrap();
    let outcome = tendril_core::jobs::deliverable::verify_deliverable(&w.paths.plans_dir, &mut job, &[]);
    assert!(matches!(outcome, tendril_core::jobs::deliverable::Deliverable::Missing { .. }));
}
