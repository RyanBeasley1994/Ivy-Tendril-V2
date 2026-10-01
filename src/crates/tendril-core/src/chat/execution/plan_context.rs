//! What a chat attached to a plan is told about that plan, and where it runs.
//!
//! A chat opened beside a plan (on the plan page, or in Review) used to start in Tendril's home with
//! nothing but the conversation, so asked "why did M2 fail?" or "run the tests" it knew neither what
//! the plan was nor where its code lived. This builds the briefing it opens with — the plan, its
//! state, commits, checks and summary, and for a mission plan the goal, milestones and the decisions
//! made along the way — and picks the plan's worktree as the agent's working directory, so the code
//! the plan changed is the code in front of it.

use crate::config::{get_config_path, get_plans_dir_with_settings, load_config};
use crate::git::service::run_git;
use crate::missions::model::{mission_link, MissionRole};
use crate::missions::store::read_mission;
use crate::plans::reader::read_plan_yaml;
use crate::plans::revisions::get_revision;
use std::path::{Path, PathBuf};

/// How much of the plan text to inline. The agent can read the rest from the plan folder.
const REVISION_CHARS: usize = 6_000;
const SUMMARY_CHARS: usize = 3_000;
const LOG_ENTRIES: usize = 15;

pub struct PlanChatContext {
    /// Markdown, prepended to the turn's prompt.
    pub briefing: String,
    /// The plan's worktree, when it has one on disk.
    pub working_directory: Option<PathBuf>,
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}\n\n…(truncated; read the full file from the plan folder)")
}

/// The first worktree that exists on disk: the plan's own registered ones, then whatever is under its
/// `Worktrees/` (a mission milestone's is a link to the shared checkout), then the plan's first repo.
fn worktree_of(plan_folder: &Path, registered: &[PathBuf], repos: &[String]) -> Option<PathBuf> {
    if let Some(found) = registered.iter().find(|p| p.join(".git").exists()) {
        return Some(found.clone());
    }
    let listed = crate::git::worktree::enumerate_worktree_directories(&plan_folder.join("Worktrees"));
    if let Some(found) = listed.into_iter().next() {
        return Some(std::fs::canonicalize(&found).unwrap_or(found));
    }
    repos.iter().map(PathBuf::from).find(|p| p.is_dir())
}

pub fn plan_chat_context(tendril_home: &Path, plan_folder_name: &str) -> Option<PlanChatContext> {
    let settings = load_config(&get_config_path(tendril_home)).ok();
    let plans_dir = get_plans_dir_with_settings(tendril_home, settings.as_ref());
    let folder = plans_dir.join(plan_folder_name);
    let (plan, _) = read_plan_yaml(&folder).ok()?;

    let registered: Vec<PathBuf> = plan
        .worktrees
        .iter()
        .flatten()
        .map(|w| PathBuf::from(&w.path))
        .collect();
    let working_directory = worktree_of(&folder, &registered, &plan.repos);

    let id: String = plan_folder_name.chars().take(5).collect();
    let mut b = String::new();
    b.push_str("# Plan Context\n");
    b.push_str(&format!(
        "This chat is attached to plan #{id}, \"{}\" (project {}, state {}, level {}). Everything below is what Tendril knows about it; the plan folder is `{}`.\n\n",
        plan.title,
        plan.project,
        plan.state,
        plan.level,
        folder.display()
    ));
    match &working_directory {
        Some(dir) => {
            let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], dir)
                .ok()
                .filter(|(code, _, _)| *code == 0)
                .map(|(_, out, _)| out.trim().to_string())
                .unwrap_or_default();
            b.push_str(&format!(
                "You are running in the plan's worktree `{}`{}: this is the code the plan changed. Inspect it, run its commands and tests here. Do not reset, rebase or delete it, and ask before committing to it.\n\n",
                dir.display(),
                if branch.is_empty() { String::new() } else { format!(" on branch `{branch}`") }
            ));
            if let Ok((0, log, _)) = run_git(&["log", "--oneline", "-n", "15"], dir) {
                if !log.trim().is_empty() {
                    b.push_str("## Recent commits\n```\n");
                    b.push_str(log.trim());
                    b.push_str("\n```\n\n");
                }
            }
        }
        None => b.push_str("The plan has no worktree on disk yet, so you are not in its code.\n\n"),
    }

    if !plan.verifications.is_empty() {
        b.push_str("## Checks\n");
        for v in &plan.verifications {
            b.push_str(&format!("- {}: {}\n", v.name, v.status));
        }
        b.push_str(&format!(
            "Reports are in `{}/Verification/`. Record an outcome with `tendril plan set-verification {id} <Name> Pass|Fail`.\n\n",
            folder.display()
        ));
    }
    if !plan.commits.is_empty() {
        b.push_str(&format!("Commits recorded on the plan: {}\n\n", plan.commits.len()));
    }
    if !plan.prs.is_empty() {
        b.push_str(&format!("Pull requests: {}\n\n", plan.prs.join(", ")));
    }

    if let Ok(summary) = std::fs::read_to_string(folder.join("Artifacts").join("summary.md")) {
        if !summary.trim().is_empty() {
            b.push_str("## Execution summary\n");
            b.push_str(&clip(&summary, SUMMARY_CHARS));
            b.push_str("\n\n");
        }
    }

    if let Some(link) = mission_link(&plan) {
        if let Ok(mission) = read_mission(Path::new(&link.folder)) {
            let role = match link.role {
                MissionRole::Integration => "the mission's integration plan: its branch holds every accepted milestone and carries the one pull request".to_string(),
                MissionRole::Milestone => format!(
                    "milestone {} of the mission",
                    link.milestone.as_deref().unwrap_or("?")
                ),
            };
            b.push_str("## Mission\n");
            b.push_str(&format!(
                "This plan is {role} \"{}\" (state {}). Run `tendril mission get {}` for everything, including each milestone's tasks, contract and base commit.\n\nGoal:\n{}\n\n",
                mission.title,
                mission.state,
                Path::new(&link.folder)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.chars().take(5).collect::<String>())
                    .unwrap_or_default(),
                clip(&mission.goal, 2_000)
            ));
            b.push_str("Milestones:\n");
            for ms in &mission.milestones {
                b.push_str(&format!("- {} [{}] {}", ms.id, ms.state, ms.title));
                if let Some(summary) = &ms.summary {
                    b.push_str(&format!(" — {}", summary));
                } else if let Some(feedback) = &ms.feedback {
                    b.push_str(&format!(" — last feedback: {}", feedback));
                }
                b.push('\n');
            }
            if let Some(summary) = &mission.summary {
                b.push_str(&format!("\nMission summary: {}\n", summary));
            }
            let start = mission.log.len().saturating_sub(LOG_ENTRIES);
            if start < mission.log.len() {
                b.push_str("\nRecent mission log:\n");
                for entry in &mission.log[start..] {
                    b.push_str(&format!(
                        "- {} {}{}\n",
                        entry.at.format("%m-%d %H:%M"),
                        entry.milestone.as_deref().map(|m| format!("[{m}] ")).unwrap_or_default(),
                        entry.message
                    ));
                }
            }
            b.push('\n');
        }
    }

    // How to serve the app, so "can you run it so I can check?" is something the chat can do: the
    // project's dev-server commands (its review actions), the ports this plan was given, and how the
    // operator opens what it starts.
    let project_cfg = settings
        .as_ref()
        .and_then(|st| st.projects.iter().find(|p| p.name.eq_ignore_ascii_case(&plan.project)));
    let actions: Vec<_> = project_cfg
        .map(|p| p.review_actions.iter().filter(|a| !a.command.trim().is_empty()).collect())
        .unwrap_or_default();
    let ports = plan.allocated_ports.clone().unwrap_or_default();
    if !actions.is_empty() || !ports.is_empty() || working_directory.is_some() {
        b.push_str("## Serving the app\n");
        b.push_str(
            "You can start the project's servers so the operator can try the change. They run on the machine Tendril runs on; the operator reaches any `http://localhost:<port>` you give them through Forge's port forwarding (clicking the link forwards it automatically, and Settings → Remote Server → Port forwards opens any other port), so a remote server is fine.\n\n",
        );
        if !actions.is_empty() {
            b.push_str("The project's dev-server commands (review actions); `%PORT%`-style tokens are the plan's ports below:\n");
            for a in &actions {
                b.push_str(&format!("- **{}**: `{}`\n", a.name, a.command.trim()));
            }
            b.push('\n');
        }
        if !ports.is_empty() {
            b.push_str("Ports allocated to this plan: ");
            b.push_str(&ports.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", "));
            b.push_str("\n\n");
        }
        b.push_str(
            "To serve: run the command from the worktree in the background so this turn can finish, e.g. `nohup <command> > /tmp/<name>-<plan>.log 2>&1 &`, then wait until it is listening (`curl -s -o /dev/null -w '%{http_code}' http://localhost:<port>` or the log), and tell the operator the URL as a link: `http://localhost:<port>`. Check first whether something is already listening on the port (`lsof -i :<port>` or `ss -ltnp`) and reuse it rather than starting a second copy. Say how to stop it (the PID, or `kill $(lsof -t -i :<port>)`).\n\n",
        );
    }

    let revision = get_revision(&folder, None).unwrap_or_default();
    if !revision.trim().is_empty() {
        b.push_str("## The plan\n");
        b.push_str(&clip(&revision, REVISION_CHARS));
        b.push_str("\n\n");
    }

    // What the project has learned, ranked for this plan, so the chat knows the codebase as well as
    // the plan.
    if !plan.project.trim().is_empty() {
        let query = format!("{}\n{}", plan.title, revision);
        let paths = crate::project_memory::paths_in(&query);
        b.push_str(crate::project_memory::render_for_prompt(tendril_home, &plan.project, &query, &paths).trim_start());
        b.push_str("\n\n");
    }
    b.push_str("---\n\n");

    Some(PlanChatContext { briefing: b, working_directory })
}
