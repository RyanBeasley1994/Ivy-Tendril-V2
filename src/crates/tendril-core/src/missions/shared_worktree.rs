//! One checkout for a whole mission.
//!
//! Every milestone works in the same place: the integration plan's `Worktrees/<repo>`, on the mission
//! branch. So the worker always sees exactly what earlier milestones left, uncommitted work survives a
//! rate-limit pause, and there is never a second branch to fast-forward from. [`crate::git::worktree::add_worktree`]
//! asks [`for_milestone_plan`] first, which makes both the daemon's pre-created worktrees and the
//! promptware's `tendril plan add-worktree` land here for a milestone plan.

use crate::error::{Result, TendrilError};
use crate::git::worktree::{derive_worktree_relative_path, register_worktree, WorktreeCreation};
use crate::missions::git::ensure_branch_worktree;
use crate::missions::model::{mission_link, MissionRole};
use crate::missions::store::read_mission;
use crate::models::PlanWorktreeEntry;
use crate::plans::reader::read_plan_yaml;
use chrono::Utc;
use std::path::{Path, PathBuf};

/// Where the mission's shared checkout of `repo` is, given its integration plan folder.
pub fn shared_path(integration: &Path, repo: &Path) -> PathBuf {
    integration.join("Worktrees").join(derive_worktree_relative_path(repo))
}

/// The shared checkout for `repo`, when `plan_folder` is a mission milestone's plan; `None` for any
/// other plan, which gets its own worktree as usual.
pub fn for_milestone_plan(repo: &Path, plan_folder: &Path) -> Result<Option<WorktreeCreation>> {
    let Ok((plan, _)) = read_plan_yaml(plan_folder) else {
        return Ok(None);
    };
    let Some(link) = mission_link(&plan).filter(|l| l.role == MissionRole::Milestone) else {
        return Ok(None);
    };
    let mission = read_mission(Path::new(&link.folder))?;
    let integration_name = mission
        .integration_plan
        .clone()
        .ok_or_else(|| TendrilError::Mission("The mission has no integration plan".into()))?;
    let branch = mission
        .branch
        .clone()
        .ok_or_else(|| TendrilError::Mission("The mission has no branch".into()))?;
    let plans_dir = plan_folder
        .parent()
        .ok_or_else(|| TendrilError::Mission("A milestone plan has no Plans folder".into()))?;
    let integration = plans_dir.join(integration_name);

    let path = shared_path(&integration, repo);
    let existed = path.join(".git").exists();
    ensure_branch_worktree(repo, &path, &branch)?;
    if !existed {
        // Registered on the integration plan, which owns it; never on the milestone, so finishing a
        // milestone can never tidy the shared checkout away.
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
    link_into_plan(plan_folder, repo, &path);
    Ok(Some(WorktreeCreation {
        path,
        branch,
        repo: repo.to_path_buf(),
        reused: existed,
        shared: true,
    }))
}

/// Puts a link at the milestone plan's own `Worktrees/<repo>` pointing at the shared checkout, so a
/// promptware that looks for its worktree there (RetryPlan does) finds the mission's one checkout
/// instead of concluding there is none and cutting a new branch. Every removal path in Forge removes
/// only the link (`git::worktree::is_link`).
fn link_into_plan(plan_folder: &Path, repo: &Path, shared: &Path) {
    let link = plan_folder.join("Worktrees").join(derive_worktree_relative_path(repo));
    if crate::git::worktree::is_link(&link) || link.exists() {
        return;
    }
    if let Some(parent) = link.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    #[cfg(unix)]
    {
        if let Err(e) = std::os::unix::fs::symlink(shared, &link) {
            tracing::warn!("Could not link {} to the shared checkout: {}", link.display(), e);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = shared;
    }
}
