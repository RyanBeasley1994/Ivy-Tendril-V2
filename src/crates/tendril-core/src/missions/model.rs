//! The shape of `mission.yaml`, and the link a plan carries back to its mission.
//!
//! A mission is a goal broken into milestones that run one after another on a shared branch. The file
//! is the whole record: the driver keeps no state of its own, so a daemon restart picks a mission up
//! exactly where the file says it is.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MISSION_SCHEMA_VERSION: i32 = 1;

/// Where a mission is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MissionState {
    /// The orchestrator is breaking the goal into milestones.
    Planning,
    /// Milestones are written; the mission waits for the operator's one approval.
    AwaitingApproval,
    /// Milestones are being executed and judged, one at a time.
    Running,
    /// Every milestone passed; the orchestrator is checking the whole branch against the goal.
    Validating,
    /// Validation passed; the integration plan is in Review, waiting for its pull request.
    Review,
    /// The integration plan's pull request was created.
    Completed,
    /// Stopped for the operator: a failure the orchestrator could not recover from, a budget that ran
    /// out, or an explicit pause. `resume` picks it back up.
    Paused,
    /// The operator cancelled it.
    Cancelled,
}

impl MissionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "Planning",
            Self::AwaitingApproval => "AwaitingApproval",
            Self::Running => "Running",
            Self::Validating => "Validating",
            Self::Review => "Review",
            Self::Completed => "Completed",
            Self::Paused => "Paused",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        [
            Self::Planning,
            Self::AwaitingApproval,
            Self::Running,
            Self::Validating,
            Self::Review,
            Self::Completed,
            Self::Paused,
            Self::Cancelled,
        ]
        .into_iter()
        .find(|state| state.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// Nothing more will happen to a mission in this state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }

    /// The integration plan may leave `Blocked`: the mission has handed it over to the operator.
    pub fn releases_integration_plan(&self) -> bool {
        matches!(self, Self::Review | Self::Completed | Self::Cancelled)
    }
}

impl std::fmt::Display for MissionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where one milestone is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MilestoneState {
    Pending,
    /// Its plan is being executed (or retried).
    Executing,
    /// The orchestrator is judging the execution.
    Judging,
    /// Accepted; its commits are on the mission branch.
    Passed,
    /// Dropped by a re-plan before it passed.
    Skipped,
}

impl MilestoneState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Executing => "Executing",
            Self::Judging => "Judging",
            Self::Passed => "Passed",
            Self::Skipped => "Skipped",
        }
    }

    /// The milestone will not run again.
    pub fn is_settled(&self) -> bool {
        matches!(self, Self::Passed | Self::Skipped)
    }
}

impl std::fmt::Display for MilestoneState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn default_milestone_state() -> MilestoneState {
    MilestoneState::Pending
}

/// One milestone: a slice of the goal small enough for one `ExecutePlan` run, with the criteria the
/// orchestrator judges it against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Milestone {
    /// `M1`, `M2`, … — stable across re-plans, so the log and the plans keep pointing at the same
    /// milestone.
    pub id: String,
    pub title: String,
    /// What this milestone delivers, in a sentence or two.
    #[serde(default)]
    pub objective: String,
    /// The plan body the milestone's plan is created with: Problem / Solution / Tests, concrete
    /// enough for `ExecutePlan` to follow without researching the goal again.
    #[serde(default)]
    pub spec: String,
    /// What must be true for the orchestrator to accept it.
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default = "default_milestone_state")]
    pub state: MilestoneState,
    /// The plan folder name created for this milestone, once it has started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Executions so far: the first `ExecutePlan` plus every `RetryPlan`.
    #[serde(default)]
    pub attempts: u32,
    /// The orchestrator's last feedback on it, when it asked for a retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// The orchestrator's summary when it accepted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The commits it landed on the mission branch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<String>,
    /// Cost of every job run for it, in USD.
    #[serde(default)]
    pub cost: f64,
    /// The small, ordered steps the milestone is carried out in. The worker ticks each off with
    /// `tendril mission task` as it finishes it; the milestone is still one commit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<MilestoneTask>,
    /// What this milestone makes available to later ones: its half of the contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<ContractItem>,
    /// Names of items earlier milestones provide that this one builds on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumes: Vec<String>,
    /// Where the shared mission branch stood in each repo when the milestone first started, keyed by
    /// repo path. Its work is the diff from here, and a re-plan that drops it resets back to here.
    #[serde(rename = "baseCommits", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub base_commits: BTreeMap<String, String>,
}

/// One step of a milestone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MilestoneTask {
    /// `M2.3`: the milestone's id and the task's position.
    pub id: String,
    pub title: String,
    /// How the worker (and the judge) know it is finished.
    #[serde(rename = "doneWhen", default)]
    pub done_when: String,
    #[serde(default)]
    pub done: bool,
}

/// One thing a milestone promises later milestones: an API, a type, an endpoint, a schema, a file.
/// Later milestones name it in `consumes`, the worker must deliver it as written, and the judge
/// holds the milestone to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractItem {
    /// A unique handle, e.g. `SessionStore` or `POST /api/sessions`.
    pub name: String,
    /// `function`, `type`, `endpoint`, `schema`, `module`, `file`, `event`, `config`…
    #[serde(default)]
    pub kind: String,
    /// The exact shape: a signature, a route with its request/response, a table's columns.
    #[serde(default)]
    pub signature: String,
    /// Where it lives, when that is part of the promise (`src/auth/session.rs`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
}

impl ContractItem {
    /// One line for plans, prompts and the CLI: `SessionStore (type) at src/auth.rs: struct …`.
    pub fn describe(&self) -> String {
        let mut out = self.name.clone();
        if !self.kind.is_empty() {
            out.push_str(&format!(" ({})", self.kind));
        }
        if !self.location.is_empty() {
            out.push_str(&format!(" at {}", self.location));
        }
        if !self.signature.is_empty() {
            out.push_str(&format!(": {}", self.signature));
        }
        out
    }
}

impl Milestone {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            objective: String::new(),
            spec: String::new(),
            acceptance: Vec::new(),
            state: MilestoneState::Pending,
            plan: None,
            attempts: 0,
            feedback: None,
            summary: None,
            commits: Vec::new(),
            cost: 0.0,
            tasks: Vec::new(),
            provides: Vec::new(),
            consumes: Vec::new(),
            base_commits: BTreeMap::new(),
        }
    }
}

/// Which step of the mission a job is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MissionStep {
    /// `OrchestrateMission`, phase `Plan`.
    Plan,
    /// `ExecutePlan` on a milestone's plan.
    Execute,
    /// `RetryPlan` on a milestone's plan.
    Retry,
    /// `OrchestrateMission`, phase `Judge`.
    Judge,
    /// `OrchestrateMission`, phase `Final`.
    Final,
    /// `OrchestrateMission`, phase `Revise`: turns an operator's change request into fix-up milestones.
    Revise,
    /// `OrchestrateMission`, phase `Steer`: answers the operator's messages on a plan awaiting approval.
    Steer,
}

impl MissionStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::Execute => "Execute",
            Self::Retry => "Retry",
            Self::Judge => "Judge",
            Self::Final => "Final",
            Self::Revise => "Revise",
            Self::Steer => "Steer",
        }
    }
}

/// Which harness (coding agent) a role runs on, and optionally its model and effort. Absent fields
/// fall back to the configured default agent and its profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAgent {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl RoleAgent {
    /// Parses `agent`, `agent:model` or `agent:model:effort`. Blank parts are left unset.
    pub fn parse(spec: &str) -> Option<Self> {
        let mut parts = spec.splitn(3, ':').map(|p| p.trim().to_string());
        let agent = parts.next().filter(|a| !a.is_empty())?;
        let model = parts.next().filter(|m| !m.is_empty());
        let effort = parts.next().filter(|e| !e.is_empty());
        Some(Self { agent, model, effort })
    }

    pub fn describe(&self) -> String {
        let mut out = self.agent.clone();
        if let Some(m) = &self.model {
            out.push_str(&format!(" · {}", m));
        }
        if let Some(e) = &self.effort {
            out.push_str(&format!(" · {}", e));
        }
        out
    }
}

/// The harness for each role in a mission. A role left unset runs on the configured default agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionAgents {
    /// The Plan phase: researches the goal and writes the milestones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<RoleAgent>,
    /// `ExecutePlan` and `RetryPlan` on each milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<RoleAgent>,
    /// The Judge phase: reviews each milestone against its acceptance criteria.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<RoleAgent>,
    /// The Final phase: runs the verifications on the mission branch and checks the goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<RoleAgent>,
}

impl MissionAgents {
    pub fn is_empty(&self) -> bool {
        self.planner.is_none() && self.worker.is_none() && self.judge.is_none() && self.validator.is_none()
    }

    /// Points the role a step belongs to at `agent`.
    pub fn set_for_step(&mut self, step: MissionStep, agent: Option<RoleAgent>) {
        match step {
            MissionStep::Plan | MissionStep::Revise | MissionStep::Steer => self.planner = agent,
            MissionStep::Execute | MissionStep::Retry => self.worker = agent,
            MissionStep::Judge => self.judge = agent,
            MissionStep::Final => self.validator = agent,
        }
    }

    /// The agent a step runs on.
    pub fn for_step(&self, step: MissionStep) -> Option<&RoleAgent> {
        match step {
            MissionStep::Plan | MissionStep::Revise | MissionStep::Steer => self.planner.as_ref(),
            MissionStep::Execute | MissionStep::Retry => self.worker.as_ref(),
            MissionStep::Judge => self.judge.as_ref(),
            MissionStep::Final => self.validator.as_ref(),
        }
    }
}

/// The job the mission is waiting on. At most one at a time: milestones share a branch, so they
/// cannot run side by side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionJobRef {
    #[serde(rename = "jobId")]
    pub job_id: String,
    pub step: MissionStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    /// The harness the job was started on, when the mission chose one for its role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// What the orchestrator decided at a decision point. Written by `tendril mission decide`, consumed
/// by the driver when the orchestrator's job ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecisionAction {
    /// The milestone (or, in the final phase, the whole mission) meets its criteria.
    Accept,
    /// Run the milestone's plan again with feedback.
    Retry,
    /// The remaining milestones were rewritten; continue with the new list.
    Replan,
    /// Stop and hand the mission to the operator.
    Fail,
}

impl DecisionAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accept => "Accept",
            Self::Retry => "Retry",
            Self::Replan => "Replan",
            Self::Fail => "Fail",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        [Self::Accept, Self::Retry, Self::Replan, Self::Fail]
            .into_iter()
            .find(|a| a.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionDecision {
    pub action: DecisionAction,
    /// The job that made the decision, so a decision left over from an earlier step is never applied
    /// to a later one.
    #[serde(rename = "jobId", default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    /// Why, in the orchestrator's words. Shown in the log.
    #[serde(default)]
    pub reason: String,
    /// For `Retry`: the change request the milestone's plan is retried with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// For `Accept`: a summary of what the milestone (or mission) delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub at: DateTime<Utc>,
}

/// Limits that stop a mission running away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionBudget {
    /// Executions allowed per milestone before the mission pauses.
    #[serde(rename = "maxAttempts", default = "default_max_attempts")]
    pub max_attempts: u32,
    /// Re-plans allowed per mission before it pauses.
    #[serde(rename = "maxReplans", default = "default_max_replans")]
    pub max_replans: u32,
    /// Total spend, in USD, after which the mission pauses. `None` is no cap.
    #[serde(rename = "maxCost", default, skip_serializing_if = "Option::is_none")]
    pub max_cost: Option<f64>,
}

fn default_max_attempts() -> u32 {
    3
}

fn default_max_replans() -> u32 {
    2
}

impl Default for MissionBudget {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            max_replans: default_max_replans(),
            max_cost: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionLogEntry {
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    pub message: String,
}

fn default_schema_version() -> i32 {
    MISSION_SCHEMA_VERSION
}

fn default_mission_state() -> MissionState {
    MissionState::Planning
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionYaml {
    #[serde(rename = "schemaVersion", default = "default_schema_version")]
    pub schema_version: i32,
    pub title: String,
    /// The task as the operator stated it.
    pub goal: String,
    pub project: String,
    #[serde(default = "default_mission_state")]
    pub state: MissionState,
    /// The state to return to on `resume`, while `Paused`.
    #[serde(rename = "pausedFrom", default, skip_serializing_if = "Option::is_none")]
    pub paused_from: Option<MissionState>,
    #[serde(rename = "pauseReason", default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(rename = "approvedAt", default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    /// The plan folder name of the integration plan: the plan whose branch is the mission branch,
    /// and which carries the mission's pull request at the end.
    #[serde(rename = "integrationPlan", default, skip_serializing_if = "Option::is_none")]
    pub integration_plan: Option<String>,
    /// `tendril/<integration plan folder>`, the branch every milestone lands on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub milestones: Vec<Milestone>,
    #[serde(rename = "currentJob", default, skip_serializing_if = "Option::is_none")]
    pub current_job: Option<MissionJobRef>,
    /// Every job the mission has started, oldest first, for the UI and for cost accounting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<MissionJobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<MissionDecision>,
    #[serde(default)]
    pub budget: MissionBudget,
    /// The harness each role runs on.
    #[serde(default, skip_serializing_if = "MissionAgents::is_empty")]
    pub agents: MissionAgents,
    #[serde(default)]
    pub replans: u32,
    /// Total spend of every finished job, in USD.
    #[serde(default)]
    pub cost: f64,
    /// The orchestrator's closing summary, from the final phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<MissionLogEntry>,
    /// Set while the mission is waiting out a coding agent's rate or usage limit. The driver re-runs
    /// the interrupted step once `until` passes; waiting never uses up an attempt.
    #[serde(rename = "rateLimit", default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitWait>,
    /// Consecutive rate-limited jobs, for the back-off. Reset when a job gets through.
    #[serde(rename = "rateLimitStreak", default, skip_serializing_if = "is_zero")]
    pub rate_limit_streak: u32,
    /// What the operator asked to change after reviewing the result, oldest first. Each one sends the
    /// mission back through planning, its fix-up milestones and validation to Review.
    #[serde(rename = "changeRequests", default, skip_serializing_if = "Vec::is_empty")]
    pub change_requests: Vec<MissionChangeRequest>,
    /// The operator's messages to the orchestrator and its replies, oldest first. Read at every
    /// orchestrator phase; one on a plan awaiting approval is answered straight away (`Steer`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<OperatorMessage>,
    /// Small changes made in Review straight from the plan's chat, on the mission branch, without
    /// sending the mission back through planning.
    #[serde(rename = "quickFixes", default, skip_serializing_if = "Vec::is_empty")]
    pub quick_fixes: Vec<QuickFix>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuickFix {
    pub at: DateTime<Utc>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<String>,
}

/// A message from the operator to the orchestrator, and its answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperatorMessage {
    /// `Q1`, `Q2`, …
    pub id: String,
    pub at: DateTime<Utc>,
    pub text: String,
    /// The orchestrator's reply, once it has read and acted on the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(rename = "repliedAt", default, skip_serializing_if = "Option::is_none")]
    pub replied_at: Option<DateTime<Utc>>,
    /// What came of it, when it was not a plain reply: `change request C2`.
    #[serde(rename = "becameChangeRequest", default, skip_serializing_if = "Option::is_none")]
    pub became_change_request: Option<String>,
}

impl OperatorMessage {
    pub fn is_open(&self) -> bool {
        self.reply.is_none() && self.became_change_request.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeRequestState {
    /// Waiting for the orchestrator to plan it.
    Pending,
    /// Turned into milestones, which are running.
    Planned,
    /// Its milestones passed and the mission was validated again.
    Done,
}

/// One round of review feedback on a mission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionChangeRequest {
    /// `C1`, `C2`, …
    pub id: String,
    pub at: DateTime<Utc>,
    /// What the operator asked for, as written.
    pub text: String,
    pub state: ChangeRequestState,
    /// The milestones planned for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<String>,
    /// The milestones that already existed when it was made: anything new after it is its fix-ups.
    #[serde(rename = "existingMilestones", default, skip_serializing_if = "Vec::is_empty")]
    pub existing_milestones: Vec<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A step the mission will re-run once a rate limit has passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimitWait {
    pub until: DateTime<Utc>,
    pub step: MissionStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    /// What the agent reported, for the UI.
    #[serde(default)]
    pub reason: String,
}

impl MissionYaml {
    pub fn new(title: impl Into<String>, goal: impl Into<String>, project: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            schema_version: MISSION_SCHEMA_VERSION,
            title: title.into(),
            goal: goal.into(),
            project: project.into(),
            state: MissionState::Planning,
            paused_from: None,
            pause_reason: None,
            created: now,
            updated: now,
            approved_at: None,
            integration_plan: None,
            branch: None,
            milestones: Vec::new(),
            current_job: None,
            jobs: Vec::new(),
            decision: None,
            budget: MissionBudget::default(),
            agents: MissionAgents::default(),
            replans: 0,
            cost: 0.0,
            summary: None,
            log: Vec::new(),
            rate_limit: None,
            rate_limit_streak: 0,
            change_requests: Vec::new(),
            messages: Vec::new(),
            quick_fixes: Vec::new(),
        }
    }

    /// Messages the orchestrator has not answered yet.
    pub fn open_messages(&self) -> impl Iterator<Item = &OperatorMessage> {
        self.messages.iter().filter(|m| m.is_open())
    }

    /// The change request waiting to be planned, if any.
    pub fn pending_change_request(&self) -> Option<&MissionChangeRequest> {
        self.change_requests.iter().find(|c| c.state == ChangeRequestState::Pending)
    }

    pub fn log(&mut self, milestone: Option<&str>, message: impl Into<String>) {
        self.log.push(MissionLogEntry {
            at: Utc::now(),
            milestone: milestone.map(str::to_string),
            message: message.into(),
        });
    }

    pub fn milestone(&self, id: &str) -> Option<&Milestone> {
        self.milestones.iter().find(|m| m.id.eq_ignore_ascii_case(id))
    }

    pub fn milestone_mut(&mut self, id: &str) -> Option<&mut Milestone> {
        self.milestones
            .iter_mut()
            .find(|m| m.id.eq_ignore_ascii_case(id))
    }

    /// The first milestone that has not settled, in order. Milestones run strictly in list order.
    pub fn next_open_milestone(&self) -> Option<&Milestone> {
        self.milestones.iter().find(|m| !m.state.is_settled())
    }

    /// The next free `M<n>` id, above every id ever used so a re-plan never reuses one.
    pub fn next_milestone_id(&self) -> String {
        let highest = self
            .milestones
            .iter()
            .filter_map(|m| m.id.strip_prefix(['M', 'm'])?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("M{}", highest + 1)
    }

    /// Stops the mission for the operator, remembering where to resume.
    pub fn pause(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if self.state != MissionState::Paused {
            self.paused_from = Some(self.state);
        }
        self.state = MissionState::Paused;
        self.log(None, format!("Paused: {}", reason));
        self.pause_reason = Some(reason);
    }

    pub fn passed_count(&self) -> usize {
        self.milestones
            .iter()
            .filter(|m| m.state == MilestoneState::Passed)
            .count()
    }
}

/// A mission as listed: the parsed file plus where it lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionFile {
    /// `00012`.
    pub id: String,
    /// `00012-AddSso`.
    #[serde(rename = "folderName")]
    pub folder_name: String,
    #[serde(rename = "folderPath")]
    pub folder_path: String,
    #[serde(flatten)]
    pub mission: MissionYaml,
}

/// The role a plan plays in a mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionRole {
    /// The plan whose branch is the mission branch.
    Integration,
    /// One milestone's plan.
    Milestone,
}

/// Stored under `mission:` in a plan's `plan.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionLink {
    /// The mission folder, absolute.
    pub folder: String,
    pub role: MissionRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    /// For a milestone plan: the branch its worktrees are cut from (the mission branch).
    #[serde(rename = "baseBranch", default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
}

/// The `plan.yaml` key a [`MissionLink`] is stored under. It lives in `PlanYaml::extra` rather than
/// a typed field so that a plan written by a build without missions keeps round-tripping untouched.
pub const MISSION_LINK_KEY: &str = "mission";

pub fn mission_link(plan: &crate::models::PlanYaml) -> Option<MissionLink> {
    plan.extra
        .get(MISSION_LINK_KEY)
        .and_then(|v| serde_yaml::from_value(v.clone()).ok())
}

pub fn set_mission_link(plan: &mut crate::models::PlanYaml, link: &MissionLink) {
    if let Ok(value) = serde_yaml::to_value(link) {
        plan.extra.insert(MISSION_LINK_KEY.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_milestone_id_never_reuses_an_id() {
        let mut m = MissionYaml::new("t", "g", "p");
        assert_eq!(m.next_milestone_id(), "M1");
        m.milestones.push(Milestone::new("M1", "a"));
        m.milestones.push(Milestone::new("M4", "b"));
        assert_eq!(m.next_milestone_id(), "M5");
    }

    #[test]
    fn pause_remembers_the_state_to_resume_to_and_does_not_overwrite_it() {
        let mut m = MissionYaml::new("t", "g", "p");
        m.state = MissionState::Running;
        m.pause("first");
        m.pause("second");
        assert_eq!(m.state, MissionState::Paused);
        assert_eq!(m.paused_from, Some(MissionState::Running));
        assert_eq!(m.pause_reason.as_deref(), Some("second"));
    }

    #[test]
    fn mission_link_round_trips_through_plan_extra() {
        let mut plan = crate::models::PlanYaml::default();
        assert!(mission_link(&plan).is_none());
        let link = MissionLink {
            folder: "/x/Missions/00001-A".into(),
            role: MissionRole::Milestone,
            milestone: Some("M2".into()),
            base_branch: Some("tendril/00007-A".into()),
        };
        set_mission_link(&mut plan, &link);
        let raw = serde_yaml::to_string(&plan).unwrap();
        let back: crate::models::PlanYaml = serde_yaml::from_str(&raw).unwrap();
        assert_eq!(mission_link(&back), Some(link));
    }

    #[test]
    fn role_agents_parse_and_map_to_steps() {
        assert_eq!(RoleAgent::parse(""), None);
        assert_eq!(
            RoleAgent::parse("codex:gpt-5.6-sol:high"),
            Some(RoleAgent { agent: "codex".into(), model: Some("gpt-5.6-sol".into()), effort: Some("high".into()) })
        );
        assert_eq!(RoleAgent::parse("claude::low").unwrap().model, None);
        let agents = MissionAgents { worker: RoleAgent::parse("codex"), ..Default::default() };
        assert_eq!(agents.for_step(MissionStep::Retry).unwrap().agent, "codex");
        assert!(agents.for_step(MissionStep::Judge).is_none());
    }

    #[test]
    fn states_parse_case_insensitively() {
        assert_eq!(
            MissionState::from_str_loose("awaitingapproval"),
            Some(MissionState::AwaitingApproval)
        );
        assert_eq!(DecisionAction::from_str_loose(" REPLAN "), Some(DecisionAction::Replan));
    }
}
