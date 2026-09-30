//! The mission driver: reads a mission's file, looks at the job it is waiting on, and takes the next
//! step.
//!
//! It is a reconciler, not an event handler. Every call to [`MissionDriver::reconcile`] derives what
//! to do from `mission.yaml` and the job table alone, so it is safe to call as often as anyone likes:
//! on every job event, on a timer, after a CLI or UI action, and after a restart. A call that finds
//! nothing to do writes nothing.
//!
//! The AI makes the decisions (what the milestones are, whether one passed, what to change); the
//! driver only carries them out, which is what keeps a mission alive across agent crashes, job
//! timeouts and daemon restarts.

use crate::config::TendrilSettings;
use crate::error::{Result, TendrilError};
use crate::git::worktree::{cleanup_worktrees, derive_branch_name, derive_worktree_relative_path, register_worktree};
use crate::jobs::firmware_values::find_project;
use crate::jobs::manager::{apply_plan_state, sync_plan_state_to_db, JobManager, StartOptions};
use crate::missions::git::{commits_between, create_mission_branch, ensure_branch_worktree, fast_forward, FastForward};
use crate::missions::model::*;
use crate::missions::service::{folder_name, integration_folder, mission_repos, MissionPaths};
use crate::missions::store::{list_missions, read_mission, update_mission};
use crate::models::{
    ExecutePlanArgs, JobArgs, JobItem, JobStatus, OrchestrateMissionArgs, PlanStatus, PlanWorktreeEntry,
    RetryPlanArgs,
};
use crate::plans::reader::read_plan_yaml;
use crate::plans::revisions::write_revision;
use crate::plans::writer::{create_plan, seed_plan_from_project, write_plan_yaml, CreatePlanOptions};
use chrono::Utc;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the driver needs from the job engine. A trait so the state machine can be tested without
/// launching agents.
pub trait MissionJobs: Send + Sync {
    /// Starts a job, on `agent`'s harness when the mission chose one for the step's role.
    fn start<'a>(&'a self, args: JobArgs, agent: Option<&'a RoleAgent>) -> BoxFuture<'a, Result<String>>;
    fn job<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<JobItem>>>;
    fn cancel<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool>>;
}

impl MissionJobs for JobManager {
    fn start<'a>(&'a self, args: JobArgs, agent: Option<&'a RoleAgent>) -> BoxFuture<'a, Result<String>> {
        let opts = StartOptions {
            agent: agent.map(|a| a.agent.clone()),
            model: agent.and_then(|a| a.model.clone()),
            effort: agent.and_then(|a| a.effort.clone()),
            ..Default::default()
        };
        Box::pin(self.start_job_with(args, opts))
    }
    fn job<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<JobItem>>> {
        Box::pin(self.get_job(id))
    }
    fn cancel<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool>> {
        Box::pin(self.cancel_job(id, Some("Mission cancelled")))
    }
}

pub type SettingsProvider = Arc<dyn Fn() -> TendrilSettings + Send + Sync>;

/// How many steps one reconcile may take before yielding. A mission rarely needs more than three in a
/// row (settle a job, start the next); the cap only exists so a bug cannot spin.
const MAX_STEPS_PER_RECONCILE: usize = 12;

pub struct MissionDriver {
    paths: MissionPaths,
    jobs: Arc<dyn MissionJobs>,
    settings: SettingsProvider,
    /// One reconcile at a time across every mission: steps start jobs and move git refs, and two
    /// interleaved steps on one mission would start its next job twice.
    lock: tokio::sync::Mutex<()>,
}

/// A job that is still going: nothing to do until it settles.
fn is_live(status: JobStatus) -> bool {
    matches!(
        status,
        JobStatus::Pending | JobStatus::Queued | JobStatus::Running | JobStatus::Blocked
    )
}

impl MissionDriver {
    pub fn new(paths: MissionPaths, jobs: Arc<dyn MissionJobs>, settings: SettingsProvider) -> Self {
        Self {
            paths,
            jobs,
            settings,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn paths(&self) -> &MissionPaths {
        &self.paths
    }

    /// Reconciles every mission that is not finished. Errors are logged per mission, never fatal.
    pub async fn reconcile_all(&self) {
        for mission in list_missions(&self.paths.missions_dir) {
            if mission.mission.state.is_terminal() {
                continue;
            }
            let folder = PathBuf::from(&mission.folder_path);
            if let Err(e) = self.reconcile(&folder).await {
                tracing::warn!("Mission {}: reconcile failed: {}", mission.folder_name, e);
            }
        }
    }

    /// Reconciles the mission a job belongs to, if it belongs to one.
    pub async fn on_job_settled(&self, job_id: &str) {
        for mission in list_missions(&self.paths.missions_dir) {
            let waiting = mission
                .mission
                .current_job
                .as_ref()
                .is_some_and(|j| j.job_id == job_id);
            if waiting {
                let folder = PathBuf::from(&mission.folder_path);
                if let Err(e) = self.reconcile(&folder).await {
                    tracing::warn!("Mission {}: reconcile failed: {}", mission.folder_name, e);
                }
            }
        }
    }

    /// Takes every step the mission is ready for. Returns whether anything changed.
    pub async fn reconcile(&self, folder: &Path) -> Result<bool> {
        let _guard = self.lock.lock().await;
        let mut changed = false;
        for _ in 0..MAX_STEPS_PER_RECONCILE {
            match self.step(folder).await {
                Ok(true) => changed = true,
                Ok(false) => return Ok(changed),
                Err(e) => {
                    // A step that cannot proceed pauses the mission with the reason, rather than
                    // failing the same way on every tick forever.
                    let message = e.to_string();
                    let _ = update_mission(folder, |m| {
                        if !m.state.is_terminal() && m.state != MissionState::Paused {
                            m.pause(message.clone());
                        }
                        Ok(())
                    });
                    return Err(e);
                }
            }
        }
        Ok(changed)
    }

    /// Cancels the mission and stops the job it was waiting on.
    pub async fn cancel(&self, folder: &Path) -> Result<()> {
        let _guard = self.lock.lock().await;
        if let Some(job) = crate::missions::service::cancel(folder)? {
            let _ = self.jobs.cancel(&job).await;
        }
        Ok(())
    }

    /// Marks the mission done and stops any job it was still waiting on.
    pub async fn complete(&self, folder: &Path) -> Result<()> {
        let _guard = self.lock.lock().await;
        if let Some(job) = crate::missions::service::complete(folder)? {
            let _ = self.jobs.cancel(&job).await;
        }
        Ok(())
    }

    /// Pauses the mission and stops the job it was waiting on, so nothing keeps spending.
    pub async fn pause(&self, folder: &Path, reason: Option<&str>) -> Result<()> {
        let _guard = self.lock.lock().await;
        let job = read_mission(folder)?.current_job.map(|j| j.job_id);
        crate::missions::service::pause(folder, reason)?;
        if let Some(job) = job {
            let _ = self.jobs.cancel(&job).await;
        }
        Ok(())
    }

    async fn step(&self, folder: &Path) -> Result<bool> {
        let mission = read_mission(folder)?;
        match mission.state {
            MissionState::AwaitingApproval
            | MissionState::Paused
            | MissionState::Completed
            | MissionState::Cancelled => Ok(false),
            MissionState::Review => self.step_review(folder, &mission),
            MissionState::Planning => self.step_planning(folder, &mission).await,
            MissionState::Running => self.step_running(folder, &mission).await,
            MissionState::Validating => self.step_validating(folder, &mission).await,
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Shared pieces
    // ---------------------------------------------------------------------------------------------

    /// The job the mission waits on, and whether it has settled. `None` when there is no current job.
    async fn current(&self, mission: &MissionYaml) -> Result<Option<(MissionJobRef, Option<JobItem>)>> {
        let Some(current) = mission.current_job.clone() else {
            return Ok(None);
        };
        let job = self.jobs.job(&current.job_id).await?;
        Ok(Some((current, job)))
    }

    async fn start_job(
        &self,
        folder: &Path,
        args: JobArgs,
        step: MissionStep,
        milestone: Option<&str>,
        message: String,
        mutate: impl FnOnce(&mut MissionYaml),
    ) -> Result<bool> {
        let agent = read_mission(folder)?.agents.for_step(step).cloned();
        let job_id = self.jobs.start(args, agent.as_ref()).await?;
        let reference = MissionJobRef {
            job_id: job_id.clone(),
            step,
            milestone: milestone.map(str::to_string),
            agent: agent.as_ref().map(RoleAgent::describe),
        };
        let message = match &agent {
            Some(a) => format!("{} on {}", message, a.describe()),
            None => message,
        };
        update_mission(folder, |m| {
            m.current_job = Some(reference.clone());
            m.jobs.push(reference.clone());
            m.decision = None;
            mutate(m);
            m.log(milestone, format!("{} (job {})", message, job_id));
            Ok(())
        })?;
        Ok(true)
    }

    fn orchestrator_args(&self, folder: &Path, mission: &MissionYaml, phase: &str, milestone: Option<&str>) -> Result<JobArgs> {
        let integration = integration_folder(&self.paths, mission)
            .ok_or_else(|| TendrilError::Mission("The mission's integration plan is missing".into()))?;
        Ok(JobArgs::OrchestrateMission(OrchestrateMissionArgs {
            folder_path: integration.to_string_lossy().to_string(),
            mission_folder: folder.to_string_lossy().to_string(),
            phase: phase.to_string(),
            milestone: milestone.map(str::to_string),
        }))
    }

    /// Clears the settled job, adds its cost, and applies the budget. Returns the mission as written.
    fn settle(folder: &Path, job: &JobItem) -> Result<MissionYaml> {
        let cost = job.cost.unwrap_or(0.0);
        update_mission(folder, |m| {
            let milestone = m.current_job.take().and_then(|j| j.milestone);
            m.cost += cost;
            if let Some(ms) = milestone.and_then(|id| m.milestone_mut(&id)) {
                ms.cost += cost;
            }
            Ok(m.clone())
        })
    }

    fn over_budget(mission: &MissionYaml) -> Option<String> {
        let cap = mission.budget.max_cost?;
        (mission.cost >= cap).then(|| format!("Cost ${:.2} reached the mission's budget of ${:.2}", mission.cost, cap))
    }

    fn job_failure(job: &JobItem) -> String {
        job.reported_failure_reason
            .clone()
            .or_else(|| job.status_message.clone())
            .unwrap_or_else(|| format!("job {} ended {}", job.id, job.status))
    }

    // ---------------------------------------------------------------------------------------------
    // Planning
    // ---------------------------------------------------------------------------------------------

    async fn step_planning(&self, folder: &Path, mission: &MissionYaml) -> Result<bool> {
        let Some((_, job)) = self.current(mission).await? else {
            let args = self.orchestrator_args(folder, mission, "Plan", None)?;
            return self
                .start_job(folder, args, MissionStep::Plan, None, "Orchestrator planning milestones".into(), |_| {})
                .await;
        };
        let Some(job) = job else {
            return self.lost_job(folder);
        };
        if is_live(job.status) {
            return Ok(false);
        }
        let mission = Self::settle(folder, &job)?;
        if job.status == JobStatus::Completed && !mission.milestones.is_empty() {
            self.publish_plan_to_integration(&mission);
            update_mission(folder, |m| {
                m.state = MissionState::AwaitingApproval;
                m.log(None, format!("{} milestones planned; waiting for approval", m.milestones.len()));
                Ok(())
            })?;
        } else {
            let reason = if job.status == JobStatus::Completed {
                "The orchestrator finished planning without writing any milestones".to_string()
            } else {
                format!("Planning failed: {}", Self::job_failure(&job))
            };
            update_mission(folder, |m| {
                m.pause(reason.clone());
                Ok(())
            })?;
        }
        Ok(true)
    }

    /// The current job's row is gone (cleared from the Jobs view, say). Forget it and redo the step.
    fn lost_job(&self, folder: &Path) -> Result<bool> {
        update_mission(folder, |m| {
            let id = m.current_job.take().map(|j| j.job_id).unwrap_or_default();
            m.log(None, format!("Job {} no longer exists; restarting the step", id));
            Ok(())
        })?;
        Ok(true)
    }

    /// Rewrites the integration plan's revision with the milestone list, so the plan reads as the
    /// mission's plan in every existing view.
    fn publish_plan_to_integration(&self, mission: &MissionYaml) {
        let Some(folder) = integration_folder(&self.paths, mission) else {
            return;
        };
        let mut body = format!("# {}\n\n## Problem\n\n{}\n\n## Solution\n\n", mission.title, mission.goal);
        body.push_str("Delivered as a mission: each milestone runs as its own plan, is judged against its acceptance criteria, and lands on this plan's branch.\n\n");
        for ms in &mission.milestones {
            body.push_str(&format!("### {} — {}\n\n", ms.id, ms.title));
            if !ms.objective.is_empty() {
                body.push_str(&format!("{}\n\n", ms.objective));
            }
            for a in &ms.acceptance {
                body.push_str(&format!("- [ ] {}\n", a));
            }
            body.push('\n');
        }
        body.push_str("## Tests\n\nEvery project verification runs against this branch once all milestones have passed, and the orchestrator checks the result against the goal.\n");
        if let Err(e) = write_revision(&folder, &body, false) {
            tracing::warn!("Could not write the mission plan to {}: {}", folder.display(), e);
        }
        sync_plan_state_to_db(&self.paths.tendril_home, &folder);
    }

    // ---------------------------------------------------------------------------------------------
    // Running
    // ---------------------------------------------------------------------------------------------

    async fn step_running(&self, folder: &Path, mission: &MissionYaml) -> Result<bool> {
        if let Some((current, job)) = self.current(mission).await? {
            let Some(job) = job else {
                return self.lost_job(folder);
            };
            if is_live(job.status) {
                return Ok(false);
            }
            let mission = Self::settle(folder, &job)?;
            let milestone = current.milestone.clone().unwrap_or_default();
            return match current.step {
                MissionStep::Execute | MissionStep::Retry => {
                    if job.status == JobStatus::Stopped {
                        update_mission(folder, |m| {
                            m.pause(format!("{} was stopped", milestone));
                            Ok(())
                        })?;
                    } else {
                        update_mission(folder, |m| {
                            if let Some(ms) = m.milestone_mut(&milestone) {
                                ms.state = MilestoneState::Judging;
                            }
                            let outcome = if job.status == JobStatus::Completed { "finished" } else { "failed" };
                            m.log(Some(&milestone), format!("Execution {}; handing to the orchestrator to judge", outcome));
                            Ok(())
                        })?;
                    }
                    Ok(true)
                }
                MissionStep::Judge => self.apply_judgement(folder, &mission, &milestone, &job).await,
                MissionStep::Plan | MissionStep::Final => {
                    // Left over from another state (a resume, a hand edit). Forget it.
                    Ok(true)
                }
            };
        }

        if let Some(reason) = Self::over_budget(mission) {
            update_mission(folder, |m| {
                m.pause(reason.clone());
                Ok(())
            })?;
            return Ok(true);
        }

        let Some(open) = mission.next_open_milestone().cloned() else {
            update_mission(folder, |m| {
                m.state = MissionState::Validating;
                m.log(None, "Every milestone passed; validating the mission branch");
                Ok(())
            })?;
            return Ok(true);
        };

        match open.state {
            MilestoneState::Pending => self.start_milestone(folder, mission, &open).await,
            // No job behind an executing or judging milestone: a resume, or a restart that lost the
            // job. The orchestrator looks at whatever is there and decides.
            MilestoneState::Executing | MilestoneState::Judging => {
                let args = self.orchestrator_args(folder, mission, "Judge", Some(&open.id))?;
                let id = open.id.clone();
                self.start_job(folder, args, MissionStep::Judge, Some(&open.id), "Orchestrator judging".into(), move |m| {
                    if let Some(ms) = m.milestone_mut(&id) {
                        ms.state = MilestoneState::Judging;
                    }
                })
                .await
            }
            MilestoneState::Passed | MilestoneState::Skipped => Ok(false),
        }
    }

    async fn start_milestone(&self, folder: &Path, mission: &MissionYaml, milestone: &Milestone) -> Result<bool> {
        self.ensure_mission_branch(mission)?;
        let plan_folder = match milestone.plan.as_ref().map(|p| self.paths.plans_dir.join(p)).filter(|p| p.is_dir()) {
            Some(existing) => existing,
            None => self.create_milestone_plan(folder, mission, milestone)?,
        };
        let plan_name = folder_name(&plan_folder);
        let branch = mission.branch.clone().unwrap_or_default();
        let note = format!(
            "This plan is milestone {} of the mission \"{}\". Its worktrees are cut from the local mission branch `{}`, which already holds every earlier milestone. Build on that work; do not push, and do not open a pull request — the mission lands everything in one pull request at the end. Acceptance criteria the orchestrator will judge this against:\n{}",
            milestone.id,
            mission.title,
            branch,
            milestone.acceptance.iter().map(|a| format!("- {}", a)).collect::<Vec<_>>().join("\n")
        );
        let args = JobArgs::ExecutePlan(ExecutePlanArgs {
            folder_path: plan_folder.to_string_lossy().to_string(),
            note: Some(note),
        });
        let id = milestone.id.clone();
        self.start_job(folder, args, MissionStep::Execute, Some(&milestone.id), format!("Executing plan {}", plan_name), move |m| {
            if let Some(ms) = m.milestone_mut(&id) {
                ms.state = MilestoneState::Executing;
                ms.plan = Some(plan_name.clone());
                ms.attempts += 1;
            }
        })
        .await
    }

    fn ensure_mission_branch(&self, mission: &MissionYaml) -> Result<()> {
        let branch = mission
            .branch
            .clone()
            .ok_or_else(|| TendrilError::Mission("The mission has no branch".into()))?;
        let settings = (self.settings)();
        let project = find_project(&settings, &mission.project);
        let integration_repos = integration_folder(&self.paths, mission)
            .and_then(|f| read_plan_yaml(&f).ok())
            .map(|(p, _)| p.repos)
            .unwrap_or_default();
        for (raw, repo) in integration_repos.iter().zip(mission_repos(&self.paths, mission)) {
            let base = project
                .and_then(|c| crate::jobs::firmware_values::find_repo_ref(c, raw))
                .and_then(|r| r.base_branch.clone());
            create_mission_branch(&repo, &branch, base.as_deref())?;
        }
        Ok(())
    }

    fn create_milestone_plan(&self, mission_folder: &Path, mission: &MissionYaml, milestone: &Milestone) -> Result<PathBuf> {
        let settings = (self.settings)();
        let project = find_project(&settings, &mission.project)
            .ok_or_else(|| TendrilError::ProjectNotFound(format!("No project named '{}'", mission.project)))?;
        let (repos, verifications) = seed_plan_from_project(project, Vec::new());
        let title = format!("{} {}", milestone.id, milestone.title);
        let mut opts = CreatePlanOptions::new(title.clone(), project.name.clone());
        opts.level = Some("Feature".to_string());
        opts.initial_prompt = Some(if milestone.objective.is_empty() {
            milestone.title.clone()
        } else {
            milestone.objective.clone()
        });
        opts.repos = repos;
        opts.verifications = verifications;
        opts.related_plans = mission.integration_plan.clone().into_iter().collect();
        let plan = create_plan(&self.paths.plans_dir, opts)?;
        let folder = PathBuf::from(&plan.folder_path);

        let (mut yaml, _) = read_plan_yaml(&folder)?;
        set_mission_link(
            &mut yaml,
            &MissionLink {
                folder: mission_folder.to_string_lossy().to_string(),
                role: MissionRole::Milestone,
                milestone: Some(milestone.id.clone()),
                base_branch: mission.branch.clone(),
            },
        );
        write_plan_yaml(&folder, &yaml)?;
        // Named now, from the template, so the name the driver fast-forwards from is the one the
        // worker's worktree was cut on.
        crate::git::branch_naming::assign_branch_name(&folder, &settings.git)?;

        let mut body = format!("# {}\n\n", title);
        body.push_str(&format!(
            "> Milestone {} of the mission **{}**. Goal: {}\n\n",
            milestone.id,
            mission.title,
            mission.goal.lines().next().unwrap_or_default()
        ));
        body.push_str(milestone.spec.trim());
        body.push_str("\n\n## Acceptance Criteria\n\n");
        for a in &milestone.acceptance {
            body.push_str(&format!("- {}\n", a));
        }
        write_revision(&folder, &body, false)?;
        sync_plan_state_to_db(&self.paths.tendril_home, &folder);
        Ok(folder)
    }

    async fn apply_judgement(&self, folder: &Path, mission: &MissionYaml, milestone_id: &str, job: &JobItem) -> Result<bool> {
        let decision = mission
            .decision
            .clone()
            .filter(|d| d.job_id.as_deref().is_none_or(|id| id == job.id));
        let Some(decision) = decision else {
            let reason = if job.status == JobStatus::Completed {
                format!("The orchestrator judged {} without recording a decision", milestone_id)
            } else {
                format!("Judging {} failed: {}", milestone_id, Self::job_failure(job))
            };
            update_mission(folder, |m| {
                m.pause(reason.clone());
                Ok(())
            })?;
            return Ok(true);
        };

        match decision.action {
            DecisionAction::Accept => self.accept_milestone(folder, mission, milestone_id, &decision),
            DecisionAction::Retry => {
                let Some(ms) = mission.milestone(milestone_id).cloned() else {
                    return Err(TendrilError::Mission(format!("Milestone {} is missing", milestone_id)));
                };
                if ms.attempts >= mission.budget.max_attempts {
                    update_mission(folder, |m| {
                        m.decision = None;
                        m.pause(format!(
                            "{} did not pass after {} attempts: {}",
                            milestone_id, ms.attempts, decision.reason
                        ));
                        Ok(())
                    })?;
                    return Ok(true);
                }
                let plan_folder = ms
                    .plan
                    .as_ref()
                    .map(|p| self.paths.plans_dir.join(p))
                    .filter(|p| p.is_dir())
                    .ok_or_else(|| TendrilError::Mission(format!("{} has no plan to retry", milestone_id)))?;
                let feedback = decision.feedback.clone().unwrap_or_else(|| decision.reason.clone());
                let args = JobArgs::RetryPlan(RetryPlanArgs {
                    folder_path: plan_folder.to_string_lossy().to_string(),
                    change_request: feedback.clone(),
                });
                let id = milestone_id.to_string();
                self.start_job(folder, args, MissionStep::Retry, Some(milestone_id), format!("Retrying (attempt {})", ms.attempts + 1), move |m| {
                    if let Some(ms) = m.milestone_mut(&id) {
                        ms.state = MilestoneState::Executing;
                        ms.attempts += 1;
                        ms.feedback = Some(feedback);
                    }
                })
                .await
            }
            DecisionAction::Replan => {
                let still_open = mission
                    .milestone(milestone_id)
                    .is_some_and(|ms| !ms.state.is_settled());
                update_mission(folder, |m| {
                    m.decision = None;
                    m.replans += 1;
                    if m.replans > m.budget.max_replans {
                        m.pause(format!("Re-plan limit of {} reached: {}", m.budget.max_replans, decision.reason));
                    } else if still_open {
                        m.pause(format!(
                            "The orchestrator decided to re-plan {} but did not rewrite the milestones",
                            milestone_id
                        ));
                    } else {
                        m.log(None, format!("Re-planned ({} of {})", m.replans, m.budget.max_replans));
                    }
                    Ok(())
                })?;
                let mission = read_mission(folder)?;
                if mission.state == MissionState::Running {
                    self.publish_plan_to_integration(&mission);
                }
                Ok(true)
            }
            DecisionAction::Fail => {
                update_mission(folder, |m| {
                    m.decision = None;
                    m.pause(format!("The orchestrator failed {}: {}", milestone_id, decision.reason));
                    Ok(())
                })?;
                Ok(true)
            }
        }
    }

    fn accept_milestone(&self, folder: &Path, mission: &MissionYaml, milestone_id: &str, decision: &MissionDecision) -> Result<bool> {
        let ms = mission
            .milestone(milestone_id)
            .cloned()
            .ok_or_else(|| TendrilError::Mission(format!("Milestone {} is missing", milestone_id)))?;
        let branch = mission
            .branch
            .clone()
            .ok_or_else(|| TendrilError::Mission("The mission has no branch".into()))?;
        let plan_folder = ms.plan.as_ref().map(|p| self.paths.plans_dir.join(p));
        let milestone_branch = plan_folder.as_deref().map(derive_branch_name).unwrap_or_default();
        let integration = integration_folder(&self.paths, mission);

        let mut commits = Vec::new();
        for repo in mission_repos(&self.paths, mission) {
            let checkout = integration
                .as_ref()
                .map(|f| f.join("Worktrees").join(derive_worktree_relative_path(&repo)));
            match fast_forward(&repo, &branch, &milestone_branch, checkout.as_deref())? {
                FastForward::Advanced { from, to } => commits.extend(commits_between(&repo, &from, &to)),
                FastForward::UpToDate | FastForward::SourceMissing => {}
            }
        }

        // The commits are on the mission branch now; the milestone's own worktrees and plan are done.
        if let Some(plan_folder) = &plan_folder {
            if let Ok((mut yaml, _)) = read_plan_yaml(plan_folder) {
                for c in &commits {
                    if !yaml.commits.contains(c) {
                        yaml.commits.push(c.clone());
                    }
                }
                let _ = write_plan_yaml(plan_folder, &yaml);
            }
            let _ = cleanup_worktrees(plan_folder);
            apply_plan_state(plan_folder, PlanStatus::Completed);
            sync_plan_state_to_db(&self.paths.tendril_home, plan_folder);
        }

        let count = commits.len();
        update_mission(folder, |m| {
            m.decision = None;
            if let Some(ms) = m.milestone_mut(milestone_id) {
                ms.state = MilestoneState::Passed;
                ms.commits.extend(commits.iter().cloned());
                ms.summary = decision.summary.clone().or_else(|| Some(decision.reason.clone()));
                ms.feedback = None;
            }
            m.log(Some(milestone_id), format!("Accepted; {} commits landed on {}", count, branch));
            Ok(())
        })?;
        Ok(true)
    }

    // ---------------------------------------------------------------------------------------------
    // Validating
    // ---------------------------------------------------------------------------------------------

    async fn step_validating(&self, folder: &Path, mission: &MissionYaml) -> Result<bool> {
        let Some((_, job)) = self.current(mission).await? else {
            if let Some(reason) = Self::over_budget(mission) {
                update_mission(folder, |m| {
                    m.pause(reason.clone());
                    Ok(())
                })?;
                return Ok(true);
            }
            self.ensure_mission_branch(mission)?;
            self.ensure_integration_worktrees(mission)?;
            let args = self.orchestrator_args(folder, mission, "Final", None)?;
            return self
                .start_job(folder, args, MissionStep::Final, None, "Orchestrator validating the mission".into(), |_| {})
                .await;
        };
        let Some(job) = job else {
            return self.lost_job(folder);
        };
        if is_live(job.status) {
            return Ok(false);
        }
        let mission = Self::settle(folder, &job)?;
        let decision = mission
            .decision
            .clone()
            .filter(|d| d.job_id.as_deref().is_none_or(|id| id == job.id));

        match decision.map(|d| (d.action, d)) {
            Some((DecisionAction::Accept, d)) => {
                self.hand_over(folder, &mission, &d)?;
                Ok(true)
            }
            Some((DecisionAction::Replan, d)) => {
                let has_open = mission.next_open_milestone().is_some();
                update_mission(folder, |m| {
                    m.decision = None;
                    m.replans += 1;
                    if m.replans > m.budget.max_replans {
                        m.pause(format!("Re-plan limit of {} reached: {}", m.budget.max_replans, d.reason));
                    } else if !has_open {
                        m.pause("The orchestrator asked for fix-up work but added no milestones");
                    } else {
                        m.state = MissionState::Running;
                        m.log(None, format!("Validation found gaps; running fix-up milestones: {}", d.reason));
                    }
                    Ok(())
                })?;
                Ok(true)
            }
            Some((_, d)) => {
                update_mission(folder, |m| {
                    m.decision = None;
                    m.pause(format!("Validation failed: {}", d.reason));
                    Ok(())
                })?;
                Ok(true)
            }
            None => {
                let reason = if job.status == JobStatus::Completed {
                    "The orchestrator validated the mission without recording a decision".to_string()
                } else {
                    format!("Validation failed: {}", Self::job_failure(&job))
                };
                update_mission(folder, |m| {
                    m.pause(reason.clone());
                    Ok(())
                })?;
                Ok(true)
            }
        }
    }

    /// Checks the mission branch out in the integration plan's `Worktrees/`, so the final phase can
    /// run verifications on it and `CreatePr` finds it where it looks for every plan's branch.
    fn ensure_integration_worktrees(&self, mission: &MissionYaml) -> Result<()> {
        let integration = integration_folder(&self.paths, mission)
            .ok_or_else(|| TendrilError::Mission("The mission's integration plan is missing".into()))?;
        let branch = mission.branch.clone().unwrap_or_else(|| derive_branch_name(&integration));
        for repo in mission_repos(&self.paths, mission) {
            let path = integration.join("Worktrees").join(derive_worktree_relative_path(&repo));
            let existed = path.join(".git").exists();
            ensure_branch_worktree(&repo, &path, &branch)?;
            if !existed {
                let _ = register_worktree(
                    &integration,
                    PlanWorktreeEntry {
                        repo: repo.to_string_lossy().to_string(),
                        path: path.to_string_lossy().to_string(),
                        branch: branch.clone(),
                        created: Utc::now(),
                    },
                );
            }
        }
        Ok(())
    }

    /// Validation passed: the integration plan goes to Review with every milestone's commits, and the
    /// operator takes it from there with the ordinary Create PR flow.
    fn hand_over(&self, folder: &Path, mission: &MissionYaml, decision: &MissionDecision) -> Result<()> {
        let summary = decision.summary.clone().unwrap_or_else(|| decision.reason.clone());
        // Released first, so the gate that holds the plan `Blocked` lets go before its state moves.
        update_mission(folder, |m| {
            m.decision = None;
            m.state = MissionState::Review;
            m.summary = Some(summary.clone());
            m.log(None, "Validated; the integration plan is ready for review");
            Ok(())
        })?;

        if let Some(integration) = integration_folder(&self.paths, mission) {
            if let Ok((mut yaml, _)) = read_plan_yaml(&integration) {
                for c in mission.milestones.iter().flat_map(|m| m.commits.iter()) {
                    if !yaml.commits.contains(c) {
                        yaml.commits.push(c.clone());
                    }
                }
                write_plan_yaml(&integration, &yaml)?;
            }
            let mut body = format!("# {}\n\n## Problem\n\n{}\n\n## Solution\n\n{}\n\n", mission.title, mission.goal, summary);
            for ms in mission.milestones.iter().filter(|m| m.state == MilestoneState::Passed) {
                body.push_str(&format!("### {} — {}\n\n", ms.id, ms.title));
                if let Some(s) = &ms.summary {
                    body.push_str(&format!("{}\n\n", s));
                }
                for a in &ms.acceptance {
                    body.push_str(&format!("- [x] {}\n", a));
                }
                body.push('\n');
            }
            body.push_str("## Tests\n\nEvery project verification was run against this branch by the mission's final validation; see `Verification/`.\n");
            let _ = write_revision(&integration, &body, false);
            apply_plan_state(&integration, PlanStatus::Review);
            sync_plan_state_to_db(&self.paths.tendril_home, &integration);
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Review
    // ---------------------------------------------------------------------------------------------

    fn step_review(&self, folder: &Path, mission: &MissionYaml) -> Result<bool> {
        let Some(integration) = integration_folder(&self.paths, mission) else {
            return Ok(false);
        };
        let Ok((plan, _)) = read_plan_yaml(&integration) else {
            return Ok(false);
        };
        if PlanStatus::from_str_loose(&plan.state) != Some(PlanStatus::Completed) {
            return Ok(false);
        }
        let prs = plan.prs.clone();
        update_mission(folder, |m| {
            m.state = MissionState::Completed;
            m.log(
                None,
                if prs.is_empty() {
                    "Completed".to_string()
                } else {
                    format!("Completed: {}", prs.join(", "))
                },
            );
            Ok(())
        })?;
        Ok(true)
    }
}
