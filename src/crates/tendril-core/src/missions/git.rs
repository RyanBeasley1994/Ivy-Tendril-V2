//! The mission branch: created once, advanced by fast-forward as each milestone is accepted.
//!
//! The branch is local only. Milestone worktrees are cut from it (`add_worktree` falls back to a
//! local base when `origin/<base>` does not exist), and it reaches GitHub only when the operator
//! creates the integration plan's pull request.

use crate::error::{Result, TendrilError};
use crate::git::service::run_git;
use std::path::Path;

/// The commit `branch` points at, if it exists.
pub fn branch_tip(repo: &Path, branch: &str) -> Option<String> {
    let reference = format!("refs/heads/{}", branch);
    match run_git(&["rev-parse", "--verify", "--quiet", &reference], repo) {
        Ok((0, out, _)) if !out.trim().is_empty() => Some(out.trim().to_string()),
        _ => None,
    }
}

pub fn branch_exists(repo: &Path, branch: &str) -> bool {
    branch_tip(repo, branch).is_some()
}

/// Creates `branch` from `origin/<base>` (or local `<base>` when there is no remote). `base` defaults
/// to the remote's HEAD, then `main`. A branch that already exists is left exactly as it is, so
/// calling this again after a restart can never rewind a mission's work.
pub fn create_mission_branch(repo: &Path, branch: &str, base: Option<&str>) -> Result<()> {
    if !repo.is_dir() {
        return Err(TendrilError::Git(format!(
            "Repo path does not exist: {}",
            repo.display()
        )));
    }
    if branch_exists(repo, branch) {
        return Ok(());
    }

    let _ = run_git(&["fetch", "origin"], repo);

    let base = match base.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => b.to_string(),
        None => match run_git(&["symbolic-ref", "refs/remotes/origin/HEAD"], repo) {
            Ok((0, out, _)) if !out.trim().is_empty() => {
                out.trim().replace("refs/remotes/origin/", "")
            }
            _ => "main".to_string(),
        },
    };

    let remote = format!("origin/{}", base);
    let (code, _, remote_err) = run_git(&["branch", branch, &remote], repo)?;
    if code == 0 {
        return Ok(());
    }
    let (code, _, local_err) = run_git(&["branch", branch, &base], repo)?;
    if code == 0 {
        return Ok(());
    }
    Err(TendrilError::Git(format!(
        "Could not create mission branch {} from {} or {}: {} {}",
        branch,
        remote,
        base,
        remote_err.trim(),
        local_err.trim()
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastForward {
    /// The target moved from the first sha to the second.
    Advanced { from: String, to: String },
    /// The source adds nothing.
    UpToDate,
    /// The source branch does not exist in this repo: the milestone never touched it.
    SourceMissing,
}

/// Moves `target` forward to `source`'s tip. Refuses anything that is not a fast-forward, because
/// that would mean the mission branch moved underneath a milestone, which sequential execution rules
/// out and which must never be papered over with a merge nobody reviewed.
///
/// When `target` is checked out in `checkout` (the integration plan's worktree), the move is done
/// there with `merge --ff-only` so the checkout follows; otherwise the ref is moved directly with a
/// compare-and-swap on its old value.
pub fn fast_forward(
    repo: &Path,
    target: &str,
    source: &str,
    checkout: Option<&Path>,
) -> Result<FastForward> {
    let Some(new) = branch_tip(repo, source) else {
        return Ok(FastForward::SourceMissing);
    };
    let Some(old) = branch_tip(repo, target) else {
        return Err(TendrilError::Git(format!(
            "Mission branch {} does not exist in {}",
            target,
            repo.display()
        )));
    };
    if old == new {
        return Ok(FastForward::UpToDate);
    }
    // `source` already contained in `target`: nothing to do (a retry that produced no new commits).
    if let Ok((0, _, _)) = run_git(&["merge-base", "--is-ancestor", &new, &old], repo) {
        return Ok(FastForward::UpToDate);
    }
    match run_git(&["merge-base", "--is-ancestor", &old, &new], repo)? {
        (0, _, _) => {}
        _ => {
            return Err(TendrilError::Git(format!(
                "{} is not a fast-forward of {} in {}; the mission branch moved while the milestone ran",
                source,
                target,
                repo.display()
            )))
        }
    }

    let checked_out = checkout.filter(|wt| wt.join(".git").exists());
    let (code, _, err) = match checked_out {
        Some(wt) => run_git(&["merge", "--ff-only", &new], wt)?,
        None => {
            let reference = format!("refs/heads/{}", target);
            run_git(&["update-ref", &reference, &new, &old], repo)?
        }
    };
    if code != 0 {
        return Err(TendrilError::Git(format!(
            "Fast-forwarding {} to {} failed: {}",
            target,
            source,
            err.trim()
        )));
    }
    Ok(FastForward::Advanced { from: old, to: new })
}

/// Commits reachable from `to` but not `from`, oldest first.
pub fn commits_between(repo: &Path, from: &str, to: &str) -> Vec<String> {
    let range = format!("{}..{}", from, to);
    match run_git(&["rev-list", "--reverse", &range], repo) {
        Ok((0, out, _)) => out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// Checks `branch` out at `path` as a worktree, without creating or resetting the branch. A path
/// that is already a worktree is left alone.
///
/// Deliberately not `add_worktree`: that cuts a *fresh* branch and deletes the old one when the
/// path is unusable, which on the mission branch would throw every accepted milestone away.
pub fn ensure_branch_worktree(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    if path.join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = run_git(&["worktree", "prune"], repo);
    let (code, _, err) = run_git(&["worktree", "add", &path.to_string_lossy(), branch], repo)?;
    if code != 0 {
        return Err(TendrilError::Git(format!(
            "Could not check out {} at {}: {}",
            branch,
            path.display(),
            err.trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git(repo: &Path, args: &[&str]) -> String {
        let (code, out, err) = run_git(args, repo).unwrap();
        assert_eq!(code, 0, "git {:?} failed: {}", args, err);
        out.trim().to_string()
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "T"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "root"]);
        (dir, repo)
    }

    fn commit_on(repo: &Path, branch: &str, msg: &str) -> String {
        git(repo, &["checkout", "-q", branch]);
        git(repo, &["commit", "-q", "--allow-empty", "-m", msg]);
        let sha = git(repo, &["rev-parse", "HEAD"]);
        git(repo, &["checkout", "-q", "main"]);
        sha
    }

    #[test]
    fn creates_once_and_never_rewinds() {
        let (_d, repo) = repo();
        create_mission_branch(&repo, "tendril/m", Some("main")).unwrap();
        let sha = commit_on(&repo, "tendril/m", "work");
        create_mission_branch(&repo, "tendril/m", Some("main")).unwrap();
        assert_eq!(branch_tip(&repo, "tendril/m").unwrap(), sha);
    }

    #[test]
    fn fast_forwards_a_milestone_and_lists_its_commits() {
        let (_d, repo) = repo();
        create_mission_branch(&repo, "tendril/m", Some("main")).unwrap();
        git(&repo, &["branch", "tendril/m1", "tendril/m"]);
        let base = branch_tip(&repo, "tendril/m").unwrap();
        let c1 = commit_on(&repo, "tendril/m1", "one");
        let c2 = commit_on(&repo, "tendril/m1", "two");

        let ff = fast_forward(&repo, "tendril/m", "tendril/m1", None).unwrap();
        assert_eq!(ff, FastForward::Advanced { from: base.clone(), to: c2.clone() });
        assert_eq!(commits_between(&repo, &base, &c2), vec![c1, c2]);
        assert_eq!(
            fast_forward(&repo, "tendril/m", "tendril/m1", None).unwrap(),
            FastForward::UpToDate
        );
        assert_eq!(
            fast_forward(&repo, "tendril/m", "tendril/nope", None).unwrap(),
            FastForward::SourceMissing
        );
    }

    #[test]
    fn refuses_a_diverged_milestone() {
        let (_d, repo) = repo();
        create_mission_branch(&repo, "tendril/m", Some("main")).unwrap();
        git(&repo, &["branch", "tendril/m1", "tendril/m"]);
        commit_on(&repo, "tendril/m1", "milestone");
        commit_on(&repo, "tendril/m", "moved underneath");
        assert!(fast_forward(&repo, "tendril/m", "tendril/m1", None).is_err());
    }

    #[test]
    fn fast_forward_moves_a_checked_out_worktree_too() {
        let (d, repo) = repo();
        create_mission_branch(&repo, "tendril/m", Some("main")).unwrap();
        let wt = d.path().join("wt");
        ensure_branch_worktree(&repo, &wt, "tendril/m").unwrap();
        ensure_branch_worktree(&repo, &wt, "tendril/m").unwrap();
        git(&repo, &["branch", "tendril/m1", "tendril/m"]);
        let c1 = commit_on(&repo, "tendril/m1", "one");
        fast_forward(&repo, "tendril/m", "tendril/m1", Some(&wt)).unwrap();
        assert_eq!(git(&wt, &["rev-parse", "HEAD"]), c1);
    }
}

/// Throws away everything in the checkout at `worktree` after `commit`: resets the branch there to it
/// and removes untracked files. Used when a re-plan drops a milestone that ran on the shared mission
/// branch, so its work does not linger under the next one.
pub fn reset_worktree(worktree: &Path, commit: &str) -> Result<()> {
    match run_git(&["reset", "--hard", commit], worktree) {
        Ok((0, _, _)) => {}
        Ok((_, _, err)) => {
            return Err(TendrilError::Git(format!(
                "Could not reset {} to {}: {}",
                worktree.display(),
                commit,
                err.trim()
            )))
        }
        Err(e) => return Err(e),
    }
    let _ = run_git(&["clean", "-fd"], worktree);
    Ok(())
}
