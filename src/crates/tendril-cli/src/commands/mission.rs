//! `tendril mission`: create and steer missions, and the two commands the orchestrator agent records
//! its work with (`set-milestones`, `decide`).
//!
//! Every write is a file edit through `tendril_core::missions::service`, so the commands work with no
//! daemon. When a daemon is running, the control commands (approve, pause, resume, cancel) go through
//! it instead, because only the daemon can stop the job a mission is waiting on; everything else
//! nudges it afterwards so the next step starts immediately rather than on the next timer tick.

use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use tendril_core::config::{get_config_path, get_plans_dir_with_settings, load_config, read_master, TendrilSettings};
use tendril_core::http::daemon_client;
use tendril_core::missions::model::{
    DecisionAction, MissionAgents, MissionFile, MissionState, MissionYaml, RoleAgent,
};
use tendril_core::missions::service::{self, MilestoneScope, MissionPaths, NewMission};
use tendril_core::missions::store::{list_missions, read_mission_file, resolve_mission_folder};

#[derive(Subcommand)]
pub enum MissionCommands {
    #[command(about = "Create a mission: a goal the orchestrator breaks into milestones")]
    Create(MissionCreateArgs),

    #[command(about = "List missions")]
    List {
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "Show a mission: goal, milestones, budget and log")]
    Get {
        id: String,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "Approve a mission's milestones; it then runs on its own")]
    Approve { id: String },

    #[command(about = "Pause a mission and stop the job it is running")]
    Pause {
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },

    #[command(about = "Resume a paused mission")]
    Resume { id: String },

    #[command(about = "Cancel a mission and stop the job it is running")]
    Cancel { id: String },

    #[command(about = "Mark a mission done: its work has landed (merged, or finished elsewhere) and it can leave the running list")]
    Complete { id: String },

    #[command(about = "Change a mission's limits")]
    Budget(MissionBudgetArgs),

    #[command(about = "Set the harness (coding agent) each role runs on")]
    Agents(MissionAgentsArgs),

    #[command(
        name = "set-milestones",
        about = "Replace a mission's milestones from a JSON or YAML file (orchestrator)"
    )]
    SetMilestones(MissionSetMilestonesArgs),

    #[command(about = "Record the orchestrator's decision for the current step (orchestrator)")]
    Decide(MissionDecideArgs),

    #[command(about = "Send a mission in Review back with changes: it plans fix-up milestones, runs and validates them, and returns to Review")]
    RequestChanges {
        id: String,
        /// The change request.
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
    },

    #[command(about = "Send the orchestrator a message: read at its next decision point (now, before approval)")]
    Message {
        id: String,
        #[arg(long)]
        text: String,
    },

    #[command(about = "Record a quick fix made in Review from the plan's chat, with its commits")]
    QuickFix {
        id: String,
        #[arg(long)]
        summary: String,
        /// A commit the fix made. Repeatable.
        #[arg(long = "commit")]
        commits: Vec<String>,
    },

    #[command(about = "Answer an operator message (orchestrator)")]
    Reply {
        id: String,
        /// The message id, e.g. Q2.
        message: String,
        #[arg(long)]
        text: String,
    },

    #[command(about = "Mark a milestone task done as you finish it, e.g. M2.3 (worker)")]
    Task {
        /// The mission id.
        id: String,
        /// The task id, e.g. M2.3.
        task: String,
        /// Mark it not done again.
        #[arg(long)]
        undo: bool,
    },
}

#[derive(Args)]
pub struct MissionCreateArgs {
    /// Short title, e.g. "Add SSO login".
    pub title: String,
    #[arg(long)]
    pub project: String,
    /// The task, in full. Use --goal-file for long text.
    #[arg(long, conflicts_with = "goal_file")]
    pub goal: Option<String>,
    #[arg(long = "goal-file")]
    pub goal_file: Option<PathBuf>,
    #[arg(long = "max-attempts", help = "Executions per milestone before pausing (default 3)")]
    pub max_attempts: Option<u32>,
    #[arg(long = "max-replans", help = "Re-plans per mission before pausing (default 2)")]
    pub max_replans: Option<u32>,
    #[arg(long = "max-cost", help = "Pause once total spend reaches this many USD")]
    pub max_cost: Option<f64>,
    #[arg(long, help = "Planner harness: agent[:model[:effort]], e.g. claude:claude-opus-5-5")]
    pub planner: Option<String>,
    #[arg(long, help = "Worker harness (ExecutePlan/RetryPlan): agent[:model[:effort]]")]
    pub worker: Option<String>,
    #[arg(long, help = "Judge harness (milestone review): agent[:model[:effort]]")]
    pub judge: Option<String>,
    #[arg(long, help = "Validator harness (final validation): agent[:model[:effort]]")]
    pub validator: Option<String>,
    #[arg(long, help = "Print JSON")]
    pub json: bool,
}

#[derive(Args)]
pub struct MissionAgentsArgs {
    pub id: String,
    #[arg(long, help = "Planner harness: agent[:model[:effort]], e.g. claude:claude-opus-5-5")]
    pub planner: Option<String>,
    #[arg(long, help = "Worker harness (ExecutePlan/RetryPlan): agent[:model[:effort]]")]
    pub worker: Option<String>,
    #[arg(long, help = "Judge harness (milestone review): agent[:model[:effort]]")]
    pub judge: Option<String>,
    #[arg(long, help = "Validator harness (final validation): agent[:model[:effort]]")]
    pub validator: Option<String>,
}

/// Builds the per-role harness from `agent[:model[:effort]]` specs. An unset role runs on the default.
fn parse_agents(
    planner: Option<&str>,
    worker: Option<&str>,
    judge: Option<&str>,
    validator: Option<&str>,
) -> MissionAgents {
    MissionAgents {
        planner: planner.and_then(RoleAgent::parse),
        worker: worker.and_then(RoleAgent::parse),
        judge: judge.and_then(RoleAgent::parse),
        validator: validator.and_then(RoleAgent::parse),
    }
}

#[derive(Args)]
pub struct MissionBudgetArgs {
    pub id: String,
    #[arg(long = "max-attempts")]
    pub max_attempts: Option<u32>,
    #[arg(long = "max-replans")]
    pub max_replans: Option<u32>,
    #[arg(long = "max-cost", help = "0 removes the cap")]
    pub max_cost: Option<f64>,
}

#[derive(Args)]
pub struct MissionSetMilestonesArgs {
    pub id: String,
    /// A list of {title, objective, spec, acceptance[], tasks[{title, doneWhen}], provides[{name, kind,
    /// signature, location}], consumes[]}, or {milestones: [...]}.
    #[arg(long)]
    pub file: PathBuf,
    /// all (before approval), pending (keep the milestone being judged) or open (drop it: a replan).
    #[arg(long, default_value = "all")]
    pub scope: String,
}

#[derive(Args)]
pub struct MissionDecideArgs {
    pub id: String,
    /// accept, retry, replan or fail.
    #[arg(long)]
    pub action: String,
    #[arg(long)]
    pub reason: String,
    /// For retry: the change request the milestone's plan is retried with.
    #[arg(long)]
    pub feedback: Option<String>,
    /// For accept: what was delivered.
    #[arg(long)]
    pub summary: Option<String>,
    /// The orchestrator's own job id (TendrilJobId), checked against the mission's current job.
    #[arg(long = "job-id")]
    pub job_id: Option<String>,
}

fn load_settings(tendril_home: &Path) -> TendrilSettings {
    load_config(&get_config_path(tendril_home)).unwrap_or_default()
}

fn paths(tendril_home: &Path, settings: &TendrilSettings) -> MissionPaths {
    MissionPaths::new(tendril_home, &get_plans_dir_with_settings(tendril_home, Some(settings)))
}

/// What asking the daemon to act on a mission came to.
enum DaemonOutcome {
    /// The daemon did it.
    Done,
    /// No daemon is running (or it could not be reached): the caller acts on the file itself.
    Unavailable,
}

/// `POST /api/missions/<folder>/<action>`. An error the daemon *answered* with is returned as an
/// error — it means the action was refused, and doing it again on the file would be wrong.
async fn ask_daemon(tendril_home: &Path, folder_name: &str, action: &str, body: serde_json::Value) -> anyhow::Result<DaemonOutcome> {
    let Some(master) = read_master(tendril_home) else {
        return Ok(DaemonOutcome::Unavailable);
    };
    let url = format!("{}/api/missions/{}/{}", master.base_url(), folder_name, action);
    let resp = match daemon_client(tendril_home).post(&url).bearer_auth(&master.secret).json(&body).send().await {
        Ok(r) => r,
        Err(_) => return Ok(DaemonOutcome::Unavailable),
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        // A daemon from before missions existed: fall back to the file.
        return Ok(DaemonOutcome::Unavailable);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or(text);
        anyhow::bail!("{} ({})", message.trim(), status);
    }
    Ok(DaemonOutcome::Done)
}

/// Nudges the daemon to take the mission's next step now. Best-effort.
async fn nudge(tendril_home: &Path, folder_name: &str) {
    let _ = ask_daemon(tendril_home, folder_name, "reconcile", serde_json::json!({})).await;
}

pub async fn handle_mission_command(cmd: MissionCommands, tendril_home: &Path) -> anyhow::Result<()> {
    let settings = load_settings(tendril_home);
    let paths = paths(tendril_home, &settings);
    let resolve = |id: &str| resolve_mission_folder(id, &paths.missions_dir);

    match cmd {
        MissionCommands::Create(args) => {
            let goal = match (args.goal, args.goal_file) {
                (Some(g), _) => g,
                (None, Some(f)) => std::fs::read_to_string(&f)?,
                (None, None) => anyhow::bail!("Pass --goal or --goal-file"),
            };
            let created = service::create(
                &paths,
                &settings,
                NewMission {
                    title: args.title,
                    goal,
                    project: args.project,
                    max_attempts: args.max_attempts,
                    max_replans: args.max_replans,
                    max_cost: args.max_cost,
                    agents: parse_agents(
                        args.planner.as_deref(),
                        args.worker.as_deref(),
                        args.judge.as_deref(),
                        args.validator.as_deref(),
                    ),
                },
            )?;
            nudge(tendril_home, &created.folder_name).await;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&created)?);
            } else {
                println!("MissionId: {}", created.id);
                println!("Folder: {}", created.folder_path);
                if let Some(plan) = &created.mission.integration_plan {
                    println!("IntegrationPlan: {}", plan);
                }
                if let Some(branch) = &created.mission.branch {
                    println!("Branch: {}", branch);
                }
                if read_master(tendril_home).is_some() {
                    println!("The orchestrator is planning milestones. Approve them with `tendril mission approve {}`.", created.id);
                } else {
                    println!("No Tendril daemon is running; planning starts when it does (`tendril run`).");
                }
            }
        }
        MissionCommands::List { json } => {
            let missions = list_missions(&paths.missions_dir);
            if json {
                println!("{}", serde_json::to_string_pretty(&missions)?);
                return Ok(());
            }
            println!("{:<6} {:<17} {:<10} {:<9} TITLE", "ID", "STATE", "PROGRESS", "COST");
            println!("{}", "-".repeat(70));
            for m in missions {
                println!(
                    "{:<6} {:<17} {:<10} {:<9} {}",
                    m.id,
                    m.mission.state.as_str(),
                    format!("{}/{}", m.mission.passed_count(), m.mission.milestones.len()),
                    format!("${:.2}", m.mission.cost),
                    m.mission.title
                );
            }
        }
        MissionCommands::Get { id, json } => {
            let file = read_mission_file(&resolve(&id)?)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&file)?);
            } else {
                print_mission(&file, &paths);
            }
        }
        MissionCommands::Approve { id } => {
            let folder = resolve(&id)?;
            let name = service::folder_name(&folder);
            if let DaemonOutcome::Unavailable = ask_daemon(tendril_home, &name, "approve", serde_json::json!({})).await? {
                service::approve(&folder)?;
            }
            println!("Approved mission {}. It runs on its own from here.", name);
        }
        MissionCommands::Pause { id, reason } => {
            let folder = resolve(&id)?;
            let name = service::folder_name(&folder);
            let body = serde_json::json!({ "reason": reason });
            if let DaemonOutcome::Unavailable = ask_daemon(tendril_home, &name, "pause", body).await? {
                service::pause(&folder, reason.as_deref())?;
            }
            println!("Paused mission {}.", name);
        }
        MissionCommands::Resume { id } => {
            let folder = resolve(&id)?;
            let name = service::folder_name(&folder);
            if let DaemonOutcome::Unavailable = ask_daemon(tendril_home, &name, "resume", serde_json::json!({})).await? {
                service::resume(&folder)?;
            }
            println!("Resumed mission {}.", name);
        }
        MissionCommands::Cancel { id } => {
            let folder = resolve(&id)?;
            let name = service::folder_name(&folder);
            if let DaemonOutcome::Unavailable = ask_daemon(tendril_home, &name, "cancel", serde_json::json!({})).await? {
                if let Some(job) = service::cancel(&folder)? {
                    println!("No daemon is running, so job {} was not stopped.", job);
                }
            }
            println!("Cancelled mission {}.", name);
        }
        MissionCommands::Complete { id } => {
            let folder = resolve(&id)?;
            let name = service::folder_name(&folder);
            if let DaemonOutcome::Unavailable = ask_daemon(tendril_home, &name, "complete", serde_json::json!({})).await? {
                if let Some(job) = service::complete(&folder)? {
                    println!("No daemon is running, so job {} was not stopped.", job);
                }
            }
            println!("Marked mission {} done.", name);
        }
        MissionCommands::Budget(args) => {
            let folder = resolve(&args.id)?;
            service::set_budget(&folder, args.max_attempts, args.max_replans, args.max_cost)?;
            nudge(tendril_home, &service::folder_name(&folder)).await;
            println!("Budget updated.");
        }
        MissionCommands::Agents(args) => {
            let folder = resolve(&args.id)?;
            let agents = parse_agents(
                args.planner.as_deref(),
                args.worker.as_deref(),
                args.judge.as_deref(),
                args.validator.as_deref(),
            );
            service::set_agents(&folder, agents)?;
            println!("Agents updated; they apply from the mission's next job.");
        }
        MissionCommands::SetMilestones(args) => {
            let folder = resolve(&args.id)?;
            let scope = MilestoneScope::from_str_loose(&args.scope)
                .ok_or_else(|| anyhow::anyhow!("--scope must be all, pending or open"))?;
            let raw = std::fs::read_to_string(&args.file)?;
            let inputs = service::parse_milestones(&raw)?;
            let ids = service::set_milestones(&folder, &paths.plans_dir, inputs, scope)?;
            println!("Milestones set: {}", ids.join(", "));
        }
        MissionCommands::RequestChanges { id, text, file } => {
            let folder = resolve(&id)?;
            let text = match (text, file) {
                (Some(t), _) => t,
                (None, Some(f)) => std::fs::read_to_string(&f)?,
                (None, None) => anyhow::bail!("Pass --text or --file"),
            };
            let cr = service::request_changes(&folder, &paths.plans_dir, &text)?;
            nudge(tendril_home, &service::folder_name(&folder)).await;
            println!("Change request {} recorded; the mission is planning it.", cr);
        }
        MissionCommands::Message { id, text } => {
            let folder = resolve(&id)?;
            let posted = service::post_message(&folder, &paths.plans_dir, &text)?;
            nudge(tendril_home, &service::folder_name(&folder)).await;
            match posted.delivery.as_str() {
                "changeRequest" => println!(
                    "The mission was in Review, so {} became change request {}.",
                    posted.id,
                    posted.change_request.unwrap_or_default()
                ),
                "immediate" => println!("Message {} sent; the orchestrator is answering it now.", posted.id),
                _ => println!("Message {} sent; the orchestrator reads it at its next decision point.", posted.id),
            }
        }
        MissionCommands::QuickFix { id, summary, commits } => {
            let folder = resolve(&id)?;
            service::record_quick_fix(&folder, &paths.plans_dir, &summary, &commits)?;
            println!("Quick fix recorded on mission {}.", service::folder_name(&folder));
        }
        MissionCommands::Reply { id, message, text } => {
            let folder = resolve(&id)?;
            service::reply_to_message(&folder, &message, &text)?;
            println!("Reply recorded on {}.", message);
        }
        MissionCommands::Task { id, task, undo } => {
            let folder = resolve(&id)?;
            service::set_task_done(&folder, &task, !undo)?;
            println!("Task {} marked {}.", task, if undo { "not done" } else { "done" });
        }
        MissionCommands::Decide(args) => {
            let folder = resolve(&args.id)?;
            let action = DecisionAction::from_str_loose(&args.action)
                .ok_or_else(|| anyhow::anyhow!("--action must be accept, retry, replan or fail"))?;
            service::decide_checked(
                &folder,
                &paths.plans_dir,
                action,
                args.job_id.as_deref(),
                &args.reason,
                args.feedback.as_deref(),
                args.summary.as_deref(),
            )?;
            println!("Decision recorded: {}. The mission continues when this job ends.", action.as_str());
        }
    }
    Ok(())
}

fn print_mission(file: &MissionFile, paths: &MissionPaths) {
    let m: &MissionYaml = &file.mission;
    println!("Mission {} — {}", file.id, m.title);
    println!("State: {}", m.state);
    if m.state == MissionState::Paused {
        if let Some(reason) = &m.pause_reason {
            println!("Paused because: {}", reason);
        }
    }
    println!("Project: {}", m.project);
    println!("Folder: {}", file.folder_path);
    if let Some(plan) = &m.integration_plan {
        println!("Integration plan: {}", paths.plans_dir.join(plan).display());
    }
    if let Some(branch) = &m.branch {
        println!("Mission branch: {} (every milestone works in the one shared worktree on it)", branch);
    }
    if let Some(wait) = &m.rate_limit {
        println!(
            "Rate limited: resuming {}{} at {} UTC ({})",
            wait.step.as_str(),
            wait.milestone.as_deref().map(|id| format!(" {}", id)).unwrap_or_default(),
            wait.until.format("%Y-%m-%d %H:%M"),
            wait.reason
        );
    }
    println!(
        "Budget: {} attempts/milestone, {} re-plans ({} used), {}",
        m.budget.max_attempts,
        m.budget.max_replans,
        m.replans,
        m.budget.max_cost.map(|c| format!("${:.2} cap", c)).unwrap_or_else(|| "no cost cap".into())
    );
    println!("Cost so far: ${:.2}", m.cost);
    let role = |r: &Option<RoleAgent>| r.as_ref().map(RoleAgent::describe).unwrap_or_else(|| "default".into());
    println!(
        "Agents: planner {}, worker {}, judge {}, validator {}",
        role(&m.agents.planner),
        role(&m.agents.worker),
        role(&m.agents.judge),
        role(&m.agents.validator)
    );
    if let Some(job) = &m.current_job {
        println!(
            "Current job: {} ({}{}{})",
            job.job_id,
            job.step.as_str(),
            job.milestone.as_deref().map(|id| format!(" {}", id)).unwrap_or_default(),
            job.agent.as_deref().map(|a| format!(" on {}", a)).unwrap_or_default()
        );
    }
    println!("\nGoal:\n{}\n", m.goal.trim());
    if !m.messages.is_empty() {
        println!("Messages:");
        for q in &m.messages {
            let status = if let Some(cr) = &q.became_change_request {
                format!("became change request {cr}")
            } else if q.reply.is_some() {
                "answered".to_string()
            } else {
                "NEEDS A REPLY".to_string()
            };
            println!("  {} [{}] {}", q.id, status, q.at.format("%Y-%m-%d %H:%M"));
            for line in q.text.lines() {
                println!("    > {}", line);
            }
            if let Some(r) = &q.reply {
                println!("    reply: {}", r);
            }
        }
        println!();
    }
    if !m.quick_fixes.is_empty() {
        println!("Quick fixes in Review:");
        for q in &m.quick_fixes {
            println!(
                "  {} {}{}",
                q.at.format("%Y-%m-%d %H:%M"),
                q.summary,
                if q.commits.is_empty() { String::new() } else { format!(" ({})", q.commits.join(", ")) }
            );
        }
        println!();
    }
    if !m.change_requests.is_empty() {
        println!("Change requests:");
        for c in &m.change_requests {
            println!(
                "  {} [{:?}] {}{}",
                c.id,
                c.state,
                c.at.format("%Y-%m-%d %H:%M"),
                if c.milestones.is_empty() { String::new() } else { format!(" -> {}", c.milestones.join(", ")) }
            );
            for line in c.text.lines() {
                println!("    {}", line);
            }
        }
        println!();
    }

    println!("Milestones ({}/{} passed):", m.passed_count(), m.milestones.len());
    for ms in &m.milestones {
        println!("\n  {} [{}] {}", ms.id, ms.state, ms.title);
        if !ms.objective.is_empty() {
            println!("    Objective: {}", ms.objective);
        }
        if let Some(plan) = &ms.plan {
            println!("    Plan: {}", paths.plans_dir.join(plan).display());
        }
        for (repo, commit) in &ms.base_commits {
            println!("    Base commit: {} in {} (this milestone's work is {}..<mission branch>)", commit, repo, &commit[..commit.len().min(12)]);
        }
        if ms.attempts > 0 {
            println!("    Attempts: {}", ms.attempts);
        }
        if !ms.tasks.is_empty() {
            println!("    Tasks:");
            for t in &ms.tasks {
                println!(
                    "      [{}] {} {}{}",
                    if t.done { "x" } else { " " },
                    t.id,
                    t.title,
                    if t.done_when.is_empty() { String::new() } else { format!(" (done when: {})", t.done_when) }
                );
            }
        }
        if !ms.consumes.is_empty() {
            println!("    Consumes: {}", ms.consumes.join(", "));
        }
        if !ms.provides.is_empty() {
            println!("    Provides:");
            for p in &ms.provides {
                println!("      - {}", p.describe());
            }
        }
        println!("    Acceptance:");
        for a in &ms.acceptance {
            println!("    - {}", a);
        }
        if let Some(f) = &ms.feedback {
            println!("    Last feedback: {}", f);
        }
        if let Some(s) = &ms.summary {
            println!("    Summary: {}", s);
        }
    }
    if let Some(s) = &m.summary {
        println!("\nSummary: {}", s);
    }
    if !m.log.is_empty() {
        println!("\nLog:");
        for entry in &m.log {
            println!(
                "  {} {}{}",
                entry.at.format("%Y-%m-%d %H:%M"),
                entry.milestone.as_deref().map(|id| format!("[{}] ", id)).unwrap_or_default(),
                entry.message
            );
        }
    }
}
