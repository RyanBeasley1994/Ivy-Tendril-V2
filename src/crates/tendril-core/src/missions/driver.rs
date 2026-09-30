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
use crate::missions::git::{branch_tip, commits_between, create_mission_branch, ensure_branch_worktree, reset_worktree};
use crate::missions::rate_limit;
use crate::missions::service::consumed_items;
use crate::missions::shared_worktree::shared_path;
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
        if let Some(wait) = mission.rate_limit.clone() {
            if matches!(mission.state, MissionState::Running | MissionState::Planning | MissionState::Validating) {
                if Utc::now() < wait.until {
                    return Ok(false);
                }
                return self.resume_after_rate_limit(folder, &mission, &wait).await;
            }
        }
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
        let limited = Self::rate_limited(job).is_some();
        update_mission(folder, |m| {
            let milestone = m.current_job.take().and_then(|j| j.milestone);
            if !limited {
                m.rate_limit_streak = 0;
            }
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

    /// What every retry of a milestone is told first: where it works and how it reports progress.
    fn retry_preamble(folder: &Path) -> String {
        let mission_id: String = folder_name(folder).chars().take(5).collect();
        format!(
            "This is a mission milestone. It works in the mission's single shared worktree on the mission branch: `Worktrees/<repo>` in this plan folder is a link to it. Use that worktree as it is. Never create, re-attach, reset or delete a worktree or branch. Tick tasks off with `tendril mission task {} <TaskId>` as you finish them (`--undo` if one turns out not done). Do not commit per task: the milestone's work is one commit, and this retry adds at most one fix-up commit on top of it once its changes are done.",
            mission_id
        )
    }

    /// The limit message, when a job that did not complete was stopped by a rate or usage limit.
    fn rate_limited(job: &JobItem) -> Option<String> {
        if matches!(job.status, JobStatus::Completed | JobStatus::Stopped) {
            return None;
        }
        rate_limit::detect(
            [job.reported_failure_reason.as_deref(), job.status_message.as_deref()]
                .into_iter()
                .flatten(),
        )
    }

    /// Parks the step to re-run after the limit, when `job` was rate limited. Returns whether it did.
    /// The milestone keeps its state and its attempt is given back: waiting is not trying.
    fn park_if_rate_limited(folder: &Path, job: &JobItem, step: MissionStep, milestone: Option<&str>) -> Result<bool> {
        let Some(reason) = Self::rate_limited(job) else {
            return Ok(false);
        };
        update_mission(folder, |m| {
            m.rate_limit_streak += 1;
            let until = rate_limit::wait_for(&reason, m.rate_limit_streak, Utc::now());
            if matches!(step, MissionStep::Execute | MissionStep::Retry) {
                if let Some(ms) = milestone.and_then(|id| m.milestone_mut(id)) {
                    ms.attempts = ms.attempts.saturating_sub(1);
                }
            }
            m.rate_limit = Some(RateLimitWait {
                until,
                step,
                milestone: milestone.map(str::to_string),
                reason: reason.clone(),
            });
            m.log(
                milestone,
                format!(
                    "Rate limited ({}); resuming {} at {} UTC",
                    reason,
                    step.as_str(),
                    until.format("%H:%M")
                ),
            );
            Ok(())
        })?;
        Ok(true)
    }

    /// The wait is over: run the interrupted step again. Plan, Judge and Final restart on their own
    /// once the wait is cleared, because each starts its job when there is none; an interrupted
    /// execution continues in the shared checkout instead of starting over.
    async fn resume_after_rate_limit(&self, folder: &Path, mission: &MissionYaml, wait: &RateLimitWait) -> Result<bool> {
        update_mission(folder, |m| {
            m.rate_limit = None;
            m.log(wait.milestone.as_deref(), format!("Rate limit wait over; resuming {}", wait.step.as_str()));
            Ok(())
        })?;
        if !matches!(wait.step, MissionStep::Execute | MissionStep::Retry) {
            return Ok(true);
        }
        let Some(ms) = wait.milestone.as_deref().and_then(|id| mission.milestone(id)).cloned() else {
            return Ok(true);
        };
        let Some(plan_folder) = ms.plan.as_ref().map(|p| self.paths.plans_dir.join(p)).filter(|p| p.is_dir()) else {
            return Ok(true);
        };
        let done: Vec<String> = ms.tasks.iter().filter(|t| t.done).map(|t| t.id.clone()).collect();
        let change_request = format!(
            "{}\n\nThe previous run of this milestone stopped early because the coding agent hit a rate or usage limit. Nothing was wrong with the work. Continue from where it stopped: the mission's shared worktree still holds everything done so far, committed or not. {} Do not redo finished work and do not reset the worktree. Finish the remaining tasks, then make the milestone's single commit.",
            Self::retry_preamble(folder),
            if done.is_empty() {
                "Check `git status` and `git diff` to see what was already done.".to_string()
            } else {
                format!("Tasks already done: {}.", done.join(", "))
            }
        );
        let args = JobArgs::RetryPlan(RetryPlanArgs {
            folder_path: plan_folder.to_string_lossy().to_string(),
            change_request,
        });
        let id = ms.id.clone();
        self.start_job(folder, args, MissionStep::Retry, Some(&ms.id), "Continuing after the rate limit".into(), move |m| {
            if let Some(ms) = m.milestone_mut(&id) {
                ms.state = MilestoneState::Executing;
                ms.attempts += 1;
            }
        })
        .await
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
        if Self::park_if_rate_limited(folder, &job, MissionStep::Plan, None)? {
            return Ok(true);
        }
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
            if !ms.tasks.is_empty() {
                body.push_str("**Tasks**\n\n");
                for t in &ms.tasks {
                    body.push_str(&format!("{}. {}\n", t.id, t.title));
                }
                body.push('\n');
            }
            if !ms.consumes.is_empty() {
                body.push_str(&format!("**Consumes:** {}\n\n", ms.consumes.iter().map(|c| format!("`{}`", c)).collect::<Vec<_>>().join(", ")));
            }
            if !ms.provides.is_empty() {
                body.push_str("**Provides**\n\n");
                for p in &ms.provides {
                    body.push_str(&format!("- `{}`: {}\n", p.name, p.describe()));
                }
                body.push('\n');
            }
            body.push_str("**Acceptance**\n\n");
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
            if matches!(current.step, MissionStep::Execute | MissionStep::Retry | MissionStep::Judge)
                && Self::park_if_rate_limited(folder, &job, current.step, Some(&milestone))?
            {
                return Ok(true);
            }
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
        // The one checkout every milestone works in, on the mission branch.
        self.ensure_integration_worktrees(mission)?;
        let branch = mission.branch.clone().unwrap_or_default();
        // Where the branch stands before this milestone touches it: its work is the diff from here,
        // and a re-plan that drops it resets back to here. Kept from the first start, so a retry
        // never moves the baseline.
        let base_commits: std::collections::BTreeMap<String, String> = if milestone.base_commits.is_empty() {
            mission_repos(&self.paths, mission)
                .into_iter()
                .filter_map(|repo| Some((repo.to_string_lossy().to_string(), branch_tip(&repo, &branch)?)))
                .collect()
        } else {
            milestone.base_commits.clone()
        };
        let plan_folder = match milestone.plan.as_ref().map(|p| self.paths.plans_dir.join(p)).filter(|p| p.is_dir()) {
            Some(existing) => existing,
            None => self.create_milestone_plan(folder, mission, milestone)?,
        };
        let plan_name = folder_name(&plan_folder);
        let mission_id = folder_name(folder).chars().take(5).collect::<String>();
        let note = format!(
            "This plan is milestone {} of the mission \"{}\". It runs in the mission's single shared worktree, on the local mission branch `{}`, which already holds every earlier milestone. Never create another worktree or branch, never reset, and never delete this one. Work through the plan's Tasks in order and run `tendril mission task {} <TaskId>` as you finish each one. Make exactly one commit for the whole milestone when every task is done. Honour the Contract section exactly. Do not push, and do not open a pull request; the mission lands everything in one pull request at the end. Acceptance criteria the orchestrator will judge this against:\n{}",
            milestone.id,
            mission.title,
            branch,
            mission_id,
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
                ms.base_commits = base_commits;
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
        let mission_id = folder_name(mission_folder).chars().take(5).collect::<String>();
        if !milestone.tasks.is_empty() {
            body.push_str("\n\n## Tasks\n\n");
            body.push_str(&format!(
                "Do these in order. When one is finished, record it with `tendril mission task {} <TaskId>`. Do **not** commit per task: make one commit for the whole milestone once every task is done.\n\n",
                mission_id
            ));
            for t in &milestone.tasks {
                body.push_str(&format!("- [ ] **{}** {}", t.id, t.title));
                if !t.done_when.is_empty() {
                    body.push_str(&format!(" _(done when: {})_", t.done_when));
                }
                body.push('\n');
            }
        }
        let consumed = consumed_items(mission, milestone);
        if !milestone.provides.is_empty() || !consumed.is_empty() {
            body.push_str("\n## Contract\n\n");
            if !consumed.is_empty() {
                body.push_str("**Consumes**: already on the branch from earlier milestones. Use these exactly as specified; do not redefine, rename or change them.\n\n");
                for (owner, item) in &consumed {
                    body.push_str(&format!("- `{}` (from {}): {}\n", item.name, owner, item.describe()));
                }
                body.push('\n');
            }
            if !milestone.provides.is_empty() {
                body.push_str("**Provides**: later milestones are written against these. Deliver each exactly as specified (name, shape, location); the orchestrator rejects the milestone if any differs.\n\n");
                for item in &milestone.provides {
                    body.push_str(&format!("- `{}`: {}\n", item.name, item.describe()));
                }
                body.push('\n');
            }
        }
        body.push_str("\n## Acceptance Criteria\n\n");
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
                    change_request: format!("{}\n\n{}", Self::retry_preamble(folder), feedback),
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
                if let Some(ms) = mission.milestone(milestone_id).filter(|ms| ms.state == MilestoneState::Skipped) {
                    self.roll_back(folder, mission, ms);
                }
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

    /// Resets the shared checkout to where the branch stood before `ms` started, so a milestone a
    /// re-plan dropped leaves nothing behind for the next one to trip over.
    fn roll_back(&self, folder: &Path, mission: &MissionYaml, ms: &Milestone) {
        let Some(integration) = integration_folder(&self.paths, mission) else { return };
        for repo in mission_repos(&self.paths, mission) {
            let Some(base) = ms.base_commits.get(&repo.to_string_lossy().to_string()) else { continue };
            let checkout = shared_path(&integration, &repo);
            let outcome = if checkout.join(".git").exists() {
                reset_worktree(&checkout, base).map(|_| format!("Rolled {} back to {}", repo.display(), &base[..base.len().min(8)]))
            } else {
                Ok(format!("No shared checkout of {}; nothing to roll back", repo.display()))
            };
            let message = outcome.unwrap_or_else(|e| format!("Could not roll back {}: {}", ms.id, e));
            let _ = update_mission(folder, |m| {
                m.log(Some(&ms.id), message.clone());
                Ok(())
            });
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

        // The milestone worked on the mission branch itself, so what it landed is everything from the
        // commit the branch stood at when it started.
        let mut commits = Vec::new();
        for repo in mission_repos(&self.paths, mission) {
            let key = repo.to_string_lossy().to_string();
            if let (Some(base), Some(tip)) = (ms.base_commits.get(&key), branch_tip(&repo, &branch)) {
                commits.extend(commits_between(&repo, base, &tip));
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
        if Self::park_if_rate_limited(folder, &job, MissionStep::Final, None)? {
            return Ok(true);
        }
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
            let mut handed_over = mission.clone();
            handed_over.summary = Some(summary.clone());
            write_integration_summary(&integration, &handed_over);
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
        // Missions handed over before the summary was written get one now. Writes nothing to the
        // mission, so it does not count as a change.
        if !integration.join("Artifacts").join("summary.md").is_file() {
            write_integration_summary(&integration, mission);
        }
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

/// `Artifacts/summary.md` for a mission's integration plan: what the Review screen shows as the
/// execution summary and what the pull request is described with. ExecutePlan writes this for an
/// ordinary plan; an integration plan is never executed, so the mission writes it on hand-over.
pub(crate) fn write_integration_summary(integration: &Path, mission: &MissionYaml) {
    let mut out = String::from("# Summary\n\n");
    if let Some(summary) = &mission.summary {
        out.push_str(summary.trim());
        out.push_str("\n\n");
    }
    out.push_str(&format!("Delivered by a mission of {} milestones.\n\n## Changes\n\n", mission.passed_count()));
    for ms in mission.milestones.iter().filter(|m| m.state == MilestoneState::Passed) {
        out.push_str(&format!("### {} — {}\n\n", ms.id, ms.title));
        if let Some(s) = ms.summary.as_ref().filter(|s| !s.trim().is_empty()) {
            out.push_str(&format!("{}\n\n", s.trim()));
        } else if !ms.objective.is_empty() {
            out.push_str(&format!("{}\n\n", ms.objective));
        }
        for a in &ms.acceptance {
            out.push_str(&format!("- [x] {}\n", a));
        }
        if !ms.commits.is_empty() {
            let short: Vec<String> = ms.commits.iter().map(|c| c.chars().take(8).collect()).collect();
            out.push_str(&format!("\nCommits: {}\n", short.join(", ")));
        }
        out.push('\n');
    }
    if let Ok((plan, _)) = read_plan_yaml(integration) {
        if !plan.verifications.is_empty() {
            out.push_str("## Verifications\n\n| Verification | Result |\n| --- | --- |\n");
            for v in &plan.verifications {
                out.push_str(&format!("| {} | {} |\n", v.name, v.status));
            }
            out.push('\n');
        }
    }
    let dir = integration.join("Artifacts");
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Err(e) = crate::fs_lock::write_atomic(&dir.join("summary.md"), out.as_bytes()) {
            tracing::warn!("Could not write the mission summary to {}: {}", dir.display(), e);
        }
    }
}
