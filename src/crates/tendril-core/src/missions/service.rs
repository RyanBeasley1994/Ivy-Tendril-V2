//! The operations on a mission that are plain file edits: creating one, approving, pausing,
//! resuming, cancelling, and the two writes the orchestrator makes (`decide`, `set-milestones`).
//!
//! Shared by the CLI (which works without a daemon) and the server routes (which follow each call
//! with a driver reconcile). None of these start jobs; that is the driver's alone.

use crate::config::{expand_config_path, TendrilSettings};
use crate::error::{Result, TendrilError};
use crate::jobs::firmware_values::find_project;
use crate::jobs::manager::{apply_plan_state, sync_plan_state_to_db};
use crate::missions::model::*;
use crate::missions::store::{create_mission, read_mission_file, update_mission, write_mission};
use crate::models::{PlanStatus, PlanVerificationEntry};
use crate::plans::reader::read_plan_yaml;
use crate::plans::revisions::write_revision;
use crate::plans::writer::{create_plan, seed_plan_from_project, write_plan_yaml, CreatePlanOptions};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where missions and plans live, plus the home for database mirroring.
#[derive(Debug, Clone)]
pub struct MissionPaths {
    pub tendril_home: PathBuf,
    pub plans_dir: PathBuf,
    pub missions_dir: PathBuf,
}

impl MissionPaths {
    pub fn new(tendril_home: &Path, plans_dir: &Path) -> Self {
        Self {
            tendril_home: tendril_home.to_path_buf(),
            plans_dir: plans_dir.to_path_buf(),
            missions_dir: crate::missions::store::missions_dir(tendril_home),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewMission {
    pub title: String,
    pub goal: String,
    pub project: String,
    #[serde(rename = "maxAttempts", default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    #[serde(rename = "maxReplans", default, skip_serializing_if = "Option::is_none")]
    pub max_replans: Option<u32>,
    #[serde(rename = "maxCost", default, skip_serializing_if = "Option::is_none")]
    pub max_cost: Option<f64>,
    /// The harness for each role; unset roles use the configured default agent.
    #[serde(default)]
    pub agents: MissionAgents,
}

/// Creates the mission and its integration plan. The integration plan is an `Epic` seeded from the
/// project like any plan, held `Blocked` by the dependency gate until the mission hands it over.
/// The mission branch itself is created at approval, when it is first needed.
pub fn create(paths: &MissionPaths, settings: &TendrilSettings, req: NewMission) -> Result<MissionFile> {
    let title = req.title.trim();
    let goal = req.goal.trim();
    if title.is_empty() {
        return Err(TendrilError::Validation("A mission needs a title".into()));
    }
    if goal.is_empty() {
        return Err(TendrilError::Validation("A mission needs a goal".into()));
    }
    let project = find_project(settings, req.project.trim()).ok_or_else(|| {
        TendrilError::ProjectNotFound(format!("No project named '{}'", req.project.trim()))
    })?;
    if project.repos.is_empty() {
        return Err(TendrilError::Validation(format!(
            "Project '{}' has no repos to run a mission in",
            project.name
        )));
    }

    let mut mission = MissionYaml::new(title, goal, project.name.clone());
    if let Some(n) = req.max_attempts {
        mission.budget.max_attempts = n.max(1);
    }
    if let Some(n) = req.max_replans {
        mission.budget.max_replans = n;
    }
    mission.budget.max_cost = req.max_cost.filter(|c| *c > 0.0);
    mission.agents = clean_agents(req.agents);
    mission.log(None, "Mission created");
    let created = create_mission(&paths.missions_dir, &mission)?;
    let mission_folder = PathBuf::from(&created.folder_path);

    let integration = match create_integration_plan(paths, project, &mission_folder, title, goal) {
        Ok(folder) => folder,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&mission_folder);
            return Err(e);
        }
    };
    let folder_name = folder_name(&integration);
    let branch = crate::git::branch_naming::assign_branch_name(&integration, &settings.git)?;
    update_mission(&mission_folder, |m| {
        m.integration_plan = Some(folder_name.clone());
        m.branch = Some(branch.clone());
        Ok(())
    })?;
    read_mission_file(&mission_folder)
}

fn create_integration_plan(
    paths: &MissionPaths,
    project: &crate::models::project::ProjectConfig,
    mission_folder: &Path,
    title: &str,
    goal: &str,
) -> Result<PathBuf> {
    let (repos, verifications) = seed_plan_from_project(project, Vec::new());
    let mut opts = CreatePlanOptions::new(title, project.name.clone());
    opts.level = Some("Epic".to_string());
    opts.initial_prompt = Some(goal.to_string());
    opts.repos = repos;
    opts.verifications = verifications;
    let plan = create_plan(&paths.plans_dir, opts)?;
    let folder = PathBuf::from(&plan.folder_path);

    let (mut yaml, _) = read_plan_yaml(&folder)?;
    yaml.state = PlanStatus::Blocked.to_string();
    set_mission_link(
        &mut yaml,
        &MissionLink {
            folder: mission_folder.to_string_lossy().to_string(),
            role: MissionRole::Integration,
            milestone: None,
            base_branch: None,
        },
    );
    write_plan_yaml(&folder, &yaml)?;
    write_revision(
        &folder,
        &format!(
            "# {}\n\n## Problem\n\n{}\n\n## Solution\n\nThis plan is a mission's integration plan. The mission's orchestrator breaks the goal into milestones, runs each one as its own plan, and lands every accepted milestone on this plan's branch. The milestones are listed here once they are planned.\n\n## Tests\n\nEvery project verification runs against this branch once all milestones have passed.\n",
            title, goal
        ),
        false,
    )?;
    sync_plan_state_to_db(&paths.tendril_home, &folder);
    Ok(folder)
}

pub fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

/// The integration plan's folder, when the mission has one.
pub fn integration_folder(paths: &MissionPaths, mission: &MissionYaml) -> Option<PathBuf> {
    mission
        .integration_plan
        .as_ref()
        .map(|name| paths.plans_dir.join(name))
        .filter(|p| p.is_dir())
}

/// The repos the mission works in, expanded: the integration plan's.
pub fn mission_repos(paths: &MissionPaths, mission: &MissionYaml) -> Vec<PathBuf> {
    integration_folder(paths, mission)
        .and_then(|f| read_plan_yaml(&f).ok())
        .map(|(plan, _)| {
            plan.repos
                .iter()
                .map(|r| expand_config_path(r, &paths.tendril_home))
                .collect()
        })
        .unwrap_or_default()
}

/// `AwaitingApproval` → `Running`. The one approval the operator gives.
pub fn approve(folder: &Path) -> Result<()> {
    update_mission(folder, |m| {
        if m.state != MissionState::AwaitingApproval {
            return Err(TendrilError::Mission(format!(
                "Only a mission awaiting approval can be approved; this one is {}",
                m.state
            )));
        }
        if m.milestones.is_empty() {
            return Err(TendrilError::Mission("The mission has no milestones to run".into()));
        }
        m.state = MissionState::Running;
        m.approved_at = Some(Utc::now());
        m.log(None, format!("Approved with {} milestones", m.milestones.len()));
        Ok(())
    })
}

pub fn pause(folder: &Path, reason: Option<&str>) -> Result<()> {
    update_mission(folder, |m| {
        if m.state.is_terminal() || m.state == MissionState::Paused {
            return Err(TendrilError::Mission(format!("Cannot pause a mission that is {}", m.state)));
        }
        m.pause(reason.filter(|r| !r.trim().is_empty()).unwrap_or("Paused by the operator"));
        Ok(())
    })
}

/// Picks a paused mission back up. The job it was waiting on is forgotten — the driver restarts the
/// right step from the file — and the open milestone gets a fresh attempt budget, because resuming
/// is the operator saying "try again".
pub fn resume(folder: &Path) -> Result<()> {
    update_mission(folder, |m| {
        if m.state != MissionState::Paused {
            return Err(TendrilError::Mission(format!("Only a paused mission can be resumed; this one is {}", m.state)));
        }
        let target = m.paused_from.take().unwrap_or(MissionState::Running);
        m.state = target;
        m.pause_reason = None;
        m.current_job = None;
        m.decision = None;
        if let Some(open) = m.milestones.iter_mut().find(|ms| !ms.state.is_settled()) {
            open.attempts = 0;
        }
        m.log(None, format!("Resumed ({})", target));
        Ok(())
    })
}

/// Marks the mission done by hand: the operator has finished it (or finished it elsewhere) and wants
/// it out of the running list. Returns the job it was waiting on, so a caller can stop it.
pub fn complete(folder: &Path) -> Result<Option<String>> {
    update_mission(folder, |m| {
        if m.state.is_terminal() {
            return Err(TendrilError::Mission(format!("The mission is already {}", m.state)));
        }
        let job = m.current_job.take().map(|j| j.job_id);
        m.state = MissionState::Completed;
        m.log(None, "Marked done by the operator");
        Ok(job)
    })
}

/// Cancels the mission. Returns the job it was waiting on, so a caller with a job manager can stop it.
pub fn cancel(folder: &Path) -> Result<Option<String>> {
    update_mission(folder, |m| {
        if m.state.is_terminal() {
            return Err(TendrilError::Mission(format!("The mission is already {}", m.state)));
        }
        let job = m.current_job.take().map(|j| j.job_id);
        m.state = MissionState::Cancelled;
        m.log(None, "Cancelled by the operator");
        Ok(job)
    })
}

pub fn set_budget(folder: &Path, max_attempts: Option<u32>, max_replans: Option<u32>, max_cost: Option<f64>) -> Result<()> {
    update_mission(folder, |m| {
        if let Some(n) = max_attempts {
            m.budget.max_attempts = n.max(1);
        }
        if let Some(n) = max_replans {
            m.budget.max_replans = n;
        }
        if let Some(c) = max_cost {
            m.budget.max_cost = (c > 0.0).then_some(c);
        }
        m.log(None, "Budget changed");
        Ok(())
    })
}

/// Drops roles with a blank agent and normalises agent ids (`ClaudeCode` → `claude`).
fn clean_agents(agents: MissionAgents) -> MissionAgents {
    let clean = |role: Option<RoleAgent>| {
        role.filter(|r| !r.agent.trim().is_empty()).map(|r| RoleAgent {
            agent: crate::agents::resolution::normalize_agent_name(&r.agent),
            model: r.model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
            effort: r.effort.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()),
        })
    };
    MissionAgents {
        planner: clean(agents.planner),
        worker: clean(agents.worker),
        judge: clean(agents.judge),
        validator: clean(agents.validator),
    }
}

/// Replaces the harness for every role. Takes effect from the next job the mission starts; the job
/// running now keeps the agent it started on.
pub fn set_agents(folder: &Path, agents: MissionAgents) -> Result<()> {
    let agents = clean_agents(agents);
    update_mission(folder, |m| {
        if m.state.is_terminal() {
            return Err(TendrilError::Mission(format!("The mission is already {}", m.state)));
        }
        let describe = |label: &str, role: &Option<RoleAgent>| {
            format!("{} {}", label, role.as_ref().map(RoleAgent::describe).unwrap_or_else(|| "default".into()))
        };
        m.log(
            None,
            format!(
                "Agents: {}, {}, {}, {}",
                describe("planner", &agents.planner),
                describe("worker", &agents.worker),
                describe("judge", &agents.judge),
                describe("validator", &agents.validator)
            ),
        );
        m.agents = agents;
        Ok(())
    })
}

/// A milestone as the orchestrator writes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MilestoneInput {
    pub title: String,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// The ordered steps; ids (`M2.1`, …) are assigned on save.
    #[serde(default)]
    pub tasks: Vec<TaskInput>,
    #[serde(default)]
    pub provides: Vec<ContractItem>,
    #[serde(default)]
    pub consumes: Vec<String>,
}

/// A task as the orchestrator writes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskInput {
    pub title: String,
    #[serde(rename = "doneWhen", alias = "done_when", default)]
    pub done_when: String,
}

/// The fewest tasks a milestone may be broken into: one task is the milestone again, not a breakdown.
pub const MIN_TASKS: usize = 2;
pub const MAX_TASKS: usize = 10;

/// Which milestones a `set-milestones` call replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilestoneScope {
    /// Every milestone. Only before approval (the Plan phase, or the operator editing the plan).
    All,
    /// Only the `Pending` ones: the milestone being judged is kept. For reshaping what comes next
    /// alongside an `accept`.
    Pending,
    /// Every milestone that has not passed, including the one being judged, which becomes
    /// `Skipped`. For a `replan`.
    Open,
}

impl MilestoneScope {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "pending" => Some(Self::Pending),
            "open" => Some(Self::Open),
            _ => None,
        }
    }
}

/// Parses milestones from JSON or YAML: a list, or `{ milestones: [...] }`.
pub fn parse_milestones(raw: &str) -> Result<Vec<MilestoneInput>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        List(Vec<MilestoneInput>),
        Wrapped { milestones: Vec<MilestoneInput> },
    }
    let shape: Shape = serde_yaml::from_str(raw)
        .map_err(|e| TendrilError::Validation(format!("Milestones are not valid JSON/YAML: {}", e)))?;
    let list = match shape {
        Shape::List(l) => l,
        Shape::Wrapped { milestones } => milestones,
    };
    for (i, m) in list.iter().enumerate() {
        if m.title.trim().is_empty() {
            return Err(TendrilError::Validation(format!("Milestone {} has no title", i + 1)));
        }
        if m.spec.trim().is_empty() {
            return Err(TendrilError::Validation(format!("Milestone '{}' has no spec", m.title)));
        }
        if m.acceptance.iter().all(|a| a.trim().is_empty()) {
            return Err(TendrilError::Validation(format!("Milestone '{}' has no acceptance criteria", m.title)));
        }
        let tasks = m.tasks.iter().filter(|t| !t.title.trim().is_empty()).count();
        if !(MIN_TASKS..=MAX_TASKS).contains(&tasks) {
            return Err(TendrilError::Validation(format!(
                "Milestone '{}' has {} tasks; break it into {}–{} ordered tasks (or split the milestone)",
                m.title, tasks, MIN_TASKS, MAX_TASKS
            )));
        }
        for item in &m.provides {
            if item.name.trim().is_empty() || item.signature.trim().is_empty() {
                return Err(TendrilError::Validation(format!(
                    "Milestone '{}' provides an item without a name and signature; every contract item needs both",
                    m.title
                )));
            }
        }
    }
    Ok(list)
}

/// Replaces milestones per `scope`. Returns the ids given to the new ones.
pub fn set_milestones(
    folder: &Path,
    plans_dir: &Path,
    inputs: Vec<MilestoneInput>,
    scope: MilestoneScope,
) -> Result<Vec<String>> {
    let skipped_plans = std::cell::RefCell::new(Vec::new());
    let ids = update_mission(folder, |m| {
        match scope {
            MilestoneScope::All => {
                if !matches!(m.state, MissionState::Planning | MissionState::AwaitingApproval) {
                    return Err(TendrilError::Mission(format!(
                        "All milestones can only be replaced before approval; the mission is {} (use --scope=pending or --scope=open)",
                        m.state
                    )));
                }
                m.milestones.clear();
            }
            MilestoneScope::Pending => m.milestones.retain(|ms| ms.state != MilestoneState::Pending),
            MilestoneScope::Open => {
                for ms in m.milestones.iter_mut() {
                    if matches!(ms.state, MilestoneState::Executing | MilestoneState::Judging) {
                        ms.state = MilestoneState::Skipped;
                        if let Some(plan) = &ms.plan {
                            skipped_plans.borrow_mut().push(plan.clone());
                        }
                    }
                }
                m.milestones.retain(|ms| ms.state != MilestoneState::Pending);
            }
        }
        let mut ids = Vec::new();
        for input in &inputs {
            let id = m.next_milestone_id();
            let mut ms = Milestone::new(id.clone(), input.title.trim());
            ms.objective = input.objective.trim().to_string();
            ms.spec = input.spec.trim().to_string();
            ms.acceptance = input
                .acceptance
                .iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            ms.tasks = input
                .tasks
                .iter()
                .filter(|t| !t.title.trim().is_empty())
                .enumerate()
                .map(|(i, t)| MilestoneTask {
                    id: format!("{}.{}", id, i + 1),
                    title: t.title.trim().to_string(),
                    done_when: t.done_when.trim().to_string(),
                    done: false,
                })
                .collect();
            ms.provides = input
                .provides
                .iter()
                .map(|c| ContractItem {
                    name: c.name.trim().to_string(),
                    kind: c.kind.trim().to_string(),
                    signature: c.signature.trim().to_string(),
                    location: c.location.trim().to_string(),
                })
                .collect();
            ms.consumes = input
                .consumes
                .iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();
            m.milestones.push(ms);
            ids.push(id);
        }
        check_contracts(&m.milestones)?;
        m.log(None, format!("Milestones set ({:?}): {}", scope, ids.join(", ")));
        Ok(ids)
    })?;

    // Outside the mission lock: a skipped milestone's plan will never run again.
    for plan in skipped_plans.into_inner() {
        let folder = plans_dir.join(plan);
        if folder.is_dir() {
            apply_plan_state(&folder, PlanStatus::Skipped);
        }
    }
    Ok(ids)
}

/// Every name a milestone consumes must be provided by a milestone before it (one that has not been
/// dropped), and no name may be provided twice. Milestones run in order on one branch, so "before"
/// is the only direction a dependency can point.
pub fn check_contracts(milestones: &[Milestone]) -> Result<()> {
    let mut provided: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for ms in milestones.iter().filter(|ms| ms.state != MilestoneState::Skipped) {
        for name in &ms.consumes {
            if !provided.contains_key(&name.to_lowercase()) {
                return Err(TendrilError::Validation(format!(
                    "{} consumes '{}', but no earlier milestone provides it. Add it to an earlier milestone's provides, or reorder.",
                    ms.id, name
                )));
            }
        }
        for item in &ms.provides {
            if let Some(owner) = provided.insert(item.name.to_lowercase(), ms.id.clone()) {
                return Err(TendrilError::Validation(format!(
                    "'{}' is provided by both {} and {}; give each contract item one owner",
                    item.name, owner, ms.id
                )));
            }
        }
    }
    Ok(())
}

/// What a milestone may rely on: every earlier, un-dropped milestone's contract item it consumes.
pub fn consumed_items(mission: &MissionYaml, milestone: &Milestone) -> Vec<(String, ContractItem)> {
    let wanted: Vec<String> = milestone.consumes.iter().map(|c| c.to_lowercase()).collect();
    mission
        .milestones
        .iter()
        .take_while(|ms| ms.id != milestone.id)
        .filter(|ms| ms.state != MilestoneState::Skipped)
        .flat_map(|ms| ms.provides.iter().map(move |p| (ms.id.clone(), p.clone())))
        .filter(|(_, p)| wanted.contains(&p.name.to_lowercase()))
        .collect()
}

/// Ticks a task off (or back on). The worker calls this as it finishes each task, which is what the
/// UI's live checklist and a resumed run after a rate limit both read.
pub fn set_task_done(folder: &Path, task_id: &str, done: bool) -> Result<()> {
    update_mission(folder, |m| {
        let task_id = task_id.trim();
        let milestone_id = task_id.split('.').next().unwrap_or_default().to_string();
        let ms = m
            .milestone_mut(&milestone_id)
            .ok_or_else(|| TendrilError::Mission(format!("No milestone {} (task ids look like M2.3)", milestone_id)))?;
        let task = ms
            .tasks
            .iter_mut()
            .find(|t| t.id.eq_ignore_ascii_case(task_id))
            .ok_or_else(|| TendrilError::Mission(format!("{} has no task {}", milestone_id, task_id)))?;
        if task.done == done {
            return Ok(());
        }
        task.done = done;
        let title = task.title.clone();
        m.log(
            Some(&milestone_id),
            format!("Task {} {}: {}", task_id, if done { "done" } else { "reopened" }, title),
        );
        Ok(())
    })
}

/// Records the orchestrator's decision for the step it is running.
#[allow(clippy::too_many_arguments)]
pub fn decide(
    folder: &Path,
    action: DecisionAction,
    job_id: Option<&str>,
    reason: &str,
    feedback: Option<&str>,
    summary: Option<&str>,
) -> Result<()> {
    update_mission(folder, |m| {
        let Some(current) = m.current_job.clone() else {
            return Err(TendrilError::Mission("The mission is not waiting on a decision".into()));
        };
        if !matches!(current.step, MissionStep::Judge | MissionStep::Final) {
            return Err(TendrilError::Mission(format!(
                "Decisions are made in the Judge and Final phases; the mission is running {}",
                current.step.as_str()
            )));
        }
        if let Some(id) = job_id.map(str::trim).filter(|s| !s.is_empty()) {
            if id != current.job_id {
                return Err(TendrilError::Mission(format!(
                    "Job {} is not the mission's current job ({})",
                    id, current.job_id
                )));
            }
        }
        if current.step == MissionStep::Final && action == DecisionAction::Retry {
            return Err(TendrilError::Mission(
                "The final phase cannot retry; use replan to add a fix-up milestone".into(),
            ));
        }
        if action == DecisionAction::Retry && feedback.is_none_or(|f| f.trim().is_empty()) {
            return Err(TendrilError::Validation("A retry needs --feedback: the change request the plan is retried with".into()));
        }
        if reason.trim().is_empty() {
            return Err(TendrilError::Validation("A decision needs a --reason".into()));
        }
        m.decision = Some(MissionDecision {
            action,
            job_id: Some(current.job_id.clone()),
            milestone: current.milestone.clone(),
            reason: reason.trim().to_string(),
            feedback: feedback.map(|f| f.trim().to_string()).filter(|f| !f.is_empty()),
            summary: summary.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            at: Utc::now(),
        });
        m.log(
            current.milestone.as_deref(),
            format!("Orchestrator decided {}: {}", action.as_str(), reason.trim()),
        );
        Ok(())
    })
}

/// [`decide`], refusing a final-phase `accept` while the integration plan still has a verification
/// `Pending` or `Fail`. The validator runs every verification in that phase; one it ran but never
/// recorded would reach Review looking unverified, which is what this catches, while the validator's
/// job is still running and can fix it.
#[allow(clippy::too_many_arguments)]
pub fn decide_checked(
    folder: &Path,
    plans_dir: &Path,
    action: DecisionAction,
    job_id: Option<&str>,
    reason: &str,
    feedback: Option<&str>,
    summary: Option<&str>,
) -> Result<()> {
    if action == DecisionAction::Accept {
        let mission = crate::missions::store::read_mission(folder)?;
        let final_phase = mission
            .current_job
            .as_ref()
            .is_some_and(|j| j.step == MissionStep::Final);
        if final_phase {
            if let Some(integration) = mission.integration_plan.as_ref().map(|p| plans_dir.join(p)) {
                if let Ok((plan, _)) = read_plan_yaml(&integration) {
                    let open = crate::plans::verification_gate::incomplete_verifications(&plan);
                    if !open.is_empty() {
                        return Err(TendrilError::Validation(format!(
                            "The integration plan still has verification(s) {} not passed. Record each result first with `tendril plan set-verification <TendrilPlanId> <Name> Pass|Fail` (and its report in Verification/<Name>.md), then accept. A failed one needs a fix-up milestone (replan), not an accept.",
                            open.join(", ")
                        )));
                    }
                }
            }
        }
    }
    decide(folder, action, job_id, reason, feedback, summary)
}

/// The operator reviewed the result and wants changes: records the request and reopens the mission.
/// The orchestrator plans fix-up milestones for it (the `Revise` phase), they run and are judged on the
/// shared branch, and the mission is validated again and comes back to Review. No fresh approval: the
/// operator asking for the change is the approval. Returns the request's id (`C1`, …).
///
/// Only from `Review` (validated, before its pull request) or a pause that came from there; the
/// integration plan goes back to `Blocked` until the mission hands it over again.
pub fn request_changes(folder: &Path, plans_dir: &Path, text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(TendrilError::Validation("A change request needs some text".into()));
    }
    let integration = std::cell::RefCell::new(None);
    let id = update_mission(folder, |m| {
        let from_review = m.state == MissionState::Review
            || (m.state == MissionState::Paused && m.paused_from == Some(MissionState::Review));
        if !from_review {
            return Err(TendrilError::Mission(format!(
                "Changes can be requested once the mission is in Review; it is {}",
                m.state
            )));
        }
        if m.current_job.is_some() {
            return Err(TendrilError::Mission("The mission is still running a job".into()));
        }
        let id = format!("C{}", m.change_requests.len() + 1);
        m.change_requests.push(MissionChangeRequest {
            id: id.clone(),
            at: Utc::now(),
            text: text.to_string(),
            state: ChangeRequestState::Pending,
            milestones: Vec::new(),
            existing_milestones: m.milestones.iter().map(|ms| ms.id.clone()).collect(),
        });
        m.state = MissionState::Planning;
        m.paused_from = None;
        m.pause_reason = None;
        m.decision = None;
        m.log(None, format!("Change request {id}: {}", text.lines().next().unwrap_or_default()));
        *integration.borrow_mut() = m.integration_plan.clone();
        Ok(id)
    })?;
    if let Some(plan) = integration.into_inner() {
        let folder = plans_dir.join(plan);
        if folder.is_dir() {
            apply_plan_state(&folder, PlanStatus::Blocked);
        }
    }
    Ok(id)
}

/// What became of an operator's message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostedMessage {
    pub id: String,
    /// `immediate` (a plan awaiting approval: the orchestrator is started on it now), `next` (read at
    /// the orchestrator's next decision point), or `changeRequest` (the mission was in Review).
    pub delivery: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_request: Option<String>,
}

/// The operator writes to the orchestrator. Every orchestrator phase reads open messages first and
/// answers each one (`tendril mission reply`), reshaping the milestones still to run if asked to; a
/// running worker is never interrupted. On a plan awaiting approval the orchestrator is started on
/// it straight away; on a mission in Review the message becomes a change request.
pub fn post_message(folder: &Path, plans_dir: &Path, text: &str) -> Result<PostedMessage> {
    let text = text.trim();
    if text.is_empty() {
        return Err(TendrilError::Validation("A message needs some text".into()));
    }
    let mission = crate::missions::store::read_mission(folder)?;
    if mission.state.is_terminal() {
        return Err(TendrilError::Mission(format!("The mission is {}; there is no one to message", mission.state)));
    }
    let in_review = mission.state == MissionState::Review
        || (mission.state == MissionState::Paused && mission.paused_from == Some(MissionState::Review));
    if in_review {
        let cr = request_changes(folder, plans_dir, text)?;
        let id = update_mission(folder, |m| {
            let id = format!("Q{}", m.messages.len() + 1);
            m.messages.push(OperatorMessage {
                id: id.clone(),
                at: Utc::now(),
                text: text.to_string(),
                reply: None,
                replied_at: None,
                became_change_request: Some(cr.clone()),
            });
            Ok(id)
        })?;
        return Ok(PostedMessage { id, delivery: "changeRequest".into(), change_request: Some(cr) });
    }
    let delivery = if mission.state == MissionState::AwaitingApproval { "immediate" } else { "next" };
    let id = update_mission(folder, |m| {
        let id = format!("Q{}", m.messages.len() + 1);
        m.messages.push(OperatorMessage {
            id: id.clone(),
            at: Utc::now(),
            text: text.to_string(),
            reply: None,
            replied_at: None,
            became_change_request: None,
        });
        m.log(None, format!("Operator message {id}: {}", text.lines().next().unwrap_or_default()));
        Ok(id)
    })?;
    Ok(PostedMessage { id, delivery: delivery.into(), change_request: None })
}

/// The orchestrator answers an operator message.
pub fn reply_to_message(folder: &Path, message_id: &str, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Err(TendrilError::Validation("A reply needs some text".into()));
    }
    update_mission(folder, |m| {
        let msg = m
            .messages
            .iter_mut()
            .find(|q| q.id.eq_ignore_ascii_case(message_id.trim()))
            .ok_or_else(|| TendrilError::Mission(format!("No message {message_id}")))?;
        msg.reply = Some(text.to_string());
        msg.replied_at = Some(Utc::now());
        let id = msg.id.clone();
        m.log(None, format!("Orchestrator replied to {id}"));
        Ok(())
    })
}

/// Records a quick fix made in Review from the plan's chat: what changed and its commits, on the
/// mission and on its integration plan (so the pull request carries them). The mission stays in
/// Review; a change that needs planning goes through [`request_changes`] instead.
pub fn record_quick_fix(folder: &Path, plans_dir: &Path, summary: &str, commits: &[String]) -> Result<()> {
    let summary = summary.trim();
    if summary.is_empty() {
        return Err(TendrilError::Validation("Say what the quick fix changed (--summary)".into()));
    }
    let integration = std::cell::RefCell::new(None);
    update_mission(folder, |m| {
        let in_review = m.state == MissionState::Review
            || (m.state == MissionState::Paused && m.paused_from == Some(MissionState::Review));
        if !in_review {
            return Err(TendrilError::Mission(format!(
                "Quick fixes are for a mission in Review; it is {}. Message the orchestrator instead.",
                m.state
            )));
        }
        m.quick_fixes.push(QuickFix {
            at: Utc::now(),
            summary: summary.to_string(),
            commits: commits.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect(),
        });
        m.log(None, format!("Quick fix in Review: {summary}"));
        *integration.borrow_mut() = m.integration_plan.clone();
        Ok(())
    })?;
    if let Some(plan) = integration.into_inner() {
        let folder = plans_dir.join(plan);
        if let Ok((mut yaml, _)) = read_plan_yaml(&folder) {
            for c in commits {
                let c = c.trim().to_string();
                if !c.is_empty() && !yaml.commits.contains(&c) {
                    yaml.commits.push(c);
                }
            }
            let _ = write_plan_yaml(&folder, &yaml);
        }
        // The pull request is written from the summary; a fix made after it was must still appear.
        let summary_path = folder.join("Artifacts").join("summary.md");
        if let Ok(mut text) = std::fs::read_to_string(&summary_path) {
            if !text.contains("## Quick fixes in review") {
                text.push_str("\n\n## Quick fixes in review\n");
            }
            text.push_str(&format!("\n- {summary}"));
            let _ = std::fs::write(&summary_path, text);
        }
    }
    Ok(())
}

/// Writes a fresh mission file, for the few callers that replace it whole.
pub fn save(folder: &Path, mission: &MissionYaml) -> Result<()> {
    write_mission(folder, mission)
}

/// The verification entries the integration plan was seeded with, for display.
pub fn integration_verifications(paths: &MissionPaths, mission: &MissionYaml) -> Vec<PlanVerificationEntry> {
    integration_folder(paths, mission)
        .and_then(|f| read_plan_yaml(&f).ok())
        .map(|(p, _)| p.verifications)
        .unwrap_or_default()
}
