//! A project's review prompt: what the AI should do when one of its plans reaches Review — start the
//! dev environment, seed test data, open the app — so it is ready by the time the operator looks.
//!
//! A timer rather than an event: a plan reaches Review by more than one road (an execution finishing,
//! a mission handing its integration plan over after its own job has ended, a manual move), and the
//! plan file is where all of them agree. Each pass looks at plans in Review whose project has a
//! `reviewPrompt`, and for any that has not been prompted for its current state of work (its latest
//! commit) it sends the prompt into the plan's chat as an event, opening that chat if there is none.
//! The chat already runs in the plan's worktree and knows how to serve the app.
//!
//! A plan already in Review when the daemon started, or since before its project had a prompt, is
//! marked and left alone: turning the setting on must not set off a turn for every plan waiting.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tendril_core::chat::execution::ChatExecutionManager;
use tendril_core::config::{get_config_path, load_config};
use tendril_core::plans::reader::read_plan_yaml;
use tendril_core::plans::writer::write_plan_yaml;

const EVERY: Duration = Duration::from_secs(15);
/// The plan.yaml key holding the state of work last prompted for.
const MARKER: &str = "reviewPromptFor";

pub fn spawn_review_prompter(tendril_home: PathBuf, plans_dir: PathBuf, chat_manager: Arc<ChatExecutionManager>) {
    tokio::spawn(async move {
        let started = chrono::Utc::now();
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = pass(&tendril_home, &plans_dir, &chat_manager, started).await {
                tracing::debug!("review prompter: {e}");
            }
        }
    });
}

async fn pass(
    tendril_home: &std::path::Path,
    plans_dir: &std::path::Path,
    chat_manager: &Arc<ChatExecutionManager>,
    started: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let settings = load_config(&get_config_path(tendril_home)).map_err(|e| e.to_string())?;
    let prompts: HashMap<String, String> = settings
        .projects
        .iter()
        .filter(|p| !p.review_prompt.trim().is_empty())
        .map(|p| (p.name.to_lowercase(), p.review_prompt.trim().to_string()))
        .collect();
    if prompts.is_empty() {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(plans_dir) else { return Ok(()) };
    for entry in entries.flatten() {
        let folder = entry.path();
        let Ok((mut plan, _)) = read_plan_yaml(&folder) else { continue };
        if plan.state != "Review" {
            continue;
        }
        let Some(prompt) = prompts.get(&plan.project.to_lowercase()) else { continue };
        // A milestone's plan is the mission's, not the operator's, to review.
        if tendril_core::missions::model::mission_link(&plan)
            .is_some_and(|l| l.role == tendril_core::missions::model::MissionRole::Milestone)
        {
            continue;
        }
        let key = format!("{}:{}", plan.commits.len(), plan.commits.last().map(String::as_str).unwrap_or("-"));
        if plan.extra.get(MARKER).and_then(|v| v.as_str()) == Some(key.as_str()) {
            continue;
        }
        let fresh = plan.updated >= started;
        plan.extra.insert(MARKER.to_string(), serde_yaml::Value::String(key));
        // Marked before anything else, so a failure below cannot make it fire on every pass.
        if write_plan_yaml(&folder, &plan).is_err() || !fresh {
            continue;
        }
        let Some(folder_name) = folder.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };

        let sessions = chat_manager.list_sessions().await.map_err(|e| e.to_string())?;
        let session_id = match sessions
            .iter()
            .filter(|s| s.plan_folder_name.as_deref() == Some(folder_name.as_str()))
            .max_by_key(|s| s.updated_at)
        {
            Some(s) => s.id.clone(),
            None => chat_manager
                .create_session(
                    Some(format!("Review: {}", plan.title)),
                    None,
                    None,
                    None,
                    Some(folder_name.clone()),
                )
                .await
                .map_err(|e| e.to_string())?
                .id,
        };
        let message = format!(
            "This plan just reached Review. The project's review instructions:\n\n{prompt}\n\nDo this now, from the plan's worktree, without asking first. Then tell the operator what is running and where (as `http://localhost:<port>` links they can open), and how to stop it."
        );
        if let Err(e) = chat_manager.notify_event(&session_id, &message).await {
            tracing::warn!("Could not send the review prompt for {folder_name}: {e}");
        } else {
            tracing::info!("Sent the review prompt for {folder_name}");
        }
    }
    Ok(())
}
