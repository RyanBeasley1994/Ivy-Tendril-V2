//! The operations a person does by hand on a repository, one per variant of [`RepoOp`].
//!
//! Each runs one git command (a few run two) built from validated pieces: a name goes through
//! [`validate_ref_name`], a revision through [`resolve_commit`], a path through [`validate_rel_path`]. A
//! failure that git explains ("not fully merged", "diverging branches") is an *outcome* with `ok: false`
//! and git's own words, because it is something to show a person, not a fault. A merge or rebase that
//! stops on conflicts is reported with the conflicted files, so the page can offer to abort or hand the
//! job to the project's manager.

use super::{
    git_network, git_read_ok, git_write, list_refs, list_worktrees, read_status, resolve_commit,
    validate_ref_name, validate_rel_path, GitOut, RefKind, RepoState,
};
use crate::error::{Result, TendrilError};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PullMode {
    /// Only if it can fast-forward: never creates a merge commit by surprise.
    #[default]
    FfOnly,
    Rebase,
    Merge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MergeMode {
    /// Fast-forward when possible, otherwise a merge commit (git's own default).
    #[default]
    Default,
    /// Always a merge commit, even when it could fast-forward.
    NoFf,
    /// Only if it can fast-forward.
    FfOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

/// `rename_all_fields` is what makes `setUpstream` and `forceWithLease` (the page's spelling) reach the
/// snake_case fields; `rename_all` alone only renames the variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RepoOp {
    Fetch {
        #[serde(default)]
        prune: bool,
    },
    Pull {
        #[serde(default)]
        mode: PullMode,
    },
    Push {
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        set_upstream: bool,
        #[serde(default)]
        force_with_lease: bool,
    },
    Checkout {
        name: String,
    },
    CheckoutRemote {
        remote_branch: String,
        #[serde(default)]
        local_name: Option<String>,
    },
    CheckoutCommit {
        hash: String,
    },
    CreateBranch {
        name: String,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        checkout: bool,
    },
    RenameBranch {
        from: String,
        to: String,
    },
    DeleteBranch {
        name: String,
        #[serde(default)]
        force: bool,
    },
    DeleteRemoteBranch {
        remote: String,
        name: String,
    },
    Stage {
        paths: Vec<String>,
    },
    StageAll,
    Unstage {
        paths: Vec<String>,
    },
    UnstageAll,
    Discard {
        paths: Vec<String>,
    },
    Commit {
        #[serde(default)]
        message: String,
        #[serde(default)]
        amend: bool,
    },
    Stash {
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        include_untracked: bool,
    },
    StashPop {
        index: u32,
    },
    StashApply {
        index: u32,
    },
    StashDrop {
        index: u32,
    },
    Merge {
        branch: String,
        #[serde(default)]
        mode: MergeMode,
    },
    MergeAbort,
    Rebase {
        onto: String,
    },
    RebaseContinue,
    RebaseSkip,
    RebaseAbort,
    CherryPick {
        hash: String,
    },
    CherryPickContinue,
    CherryPickAbort,
    Revert {
        hash: String,
    },
    RevertContinue,
    RevertAbort,
    CreateTag {
        name: String,
        #[serde(default)]
        at: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    DeleteTag {
        name: String,
    },
    Reset {
        to: String,
        mode: ResetMode,
    },
    WorktreeAdd {
        path: String,
        branch: String,
        #[serde(default)]
        create: bool,
    },
    WorktreeRemove {
        path: String,
        #[serde(default)]
        force: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpOutcome {
    pub ok: bool,
    /// Git's own words, trimmed, or a short confirmation.
    pub message: String,
    /// Files in conflict when the operation stopped on them.
    pub conflicts: Vec<String>,
    pub state: RepoState,
}

fn clip(text: &str) -> String {
    let t = text.trim();
    if t.len() <= 2000 {
        return t.to_string();
    }
    let mut cut = 2000;
    while !t.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &t[..cut])
}

/// Turns git's answer into an outcome, noticing a stop on conflicts.
fn outcome(repo: &Path, out: GitOut, done: &str) -> Result<OpOutcome> {
    let status = read_status(repo)?;
    let conflicts: Vec<String> = status.entries.iter().filter(|e| e.conflicted).map(|e| e.path.clone()).collect();
    if out.ok() {
        return Ok(OpOutcome { ok: true, message: done.to_string(), conflicts, state: status.state });
    }
    let message = if conflicts.is_empty() {
        clip(&out.message())
    } else {
        format!("Stopped on conflicts in {} file{}.", conflicts.len(), if conflicts.len() == 1 { "" } else { "s" })
    };
    Ok(OpOutcome { ok: false, message, conflicts, state: status.state })
}

fn require_state(repo: &Path, want: &[RepoState], what: &str) -> Result<()> {
    let state = read_status(repo)?.state;
    if want.contains(&state) {
        Ok(())
    } else {
        Err(TendrilError::Validation(format!("There is no {what} in progress.")))
    }
}

fn local_branch_exists(repo: &Path, name: &str) -> bool {
    list_refs(repo).map(|r| r.iter().any(|x| x.kind == RefKind::Local && x.name == name)).unwrap_or(false)
}

fn paths_ok(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err(TendrilError::Validation("No files were given.".into()));
    }
    paths.iter().try_for_each(|p| validate_rel_path(p))
}

fn is_merge_commit(repo: &Path, hash: &str) -> bool {
    git_read_ok(repo, &["rev-list", "--parents", "-n", "1", hash])
        .map(|o| o.split_whitespace().count() > 2)
        .unwrap_or(false)
}

/// Runs one operation and says what happened.
pub fn run_op(repo: &Path, op: &RepoOp) -> Result<OpOutcome> {
    match op {
        RepoOp::Fetch { prune } => {
            let mut args = vec!["fetch", "--all", "--tags"];
            if *prune {
                args.push("--prune");
            }
            outcome(repo, git_network(repo, &args)?, "Fetched.")
        }
        RepoOp::Pull { mode } => {
            let args: &[&str] = match mode {
                PullMode::FfOnly => &["pull", "--ff-only"],
                PullMode::Rebase => &["-c", "core.editor=true", "pull", "--rebase"],
                PullMode::Merge => &["-c", "core.editor=true", "pull", "--no-rebase", "--no-edit"],
            };
            outcome(repo, git_network(repo, args)?, "Pulled.")
        }
        RepoOp::Push { branch, set_upstream, force_with_lease } => {
            let status = read_status(repo)?;
            let branch = match branch.clone().or(status.branch.clone()) {
                Some(b) => b,
                None => return Err(TendrilError::Validation("HEAD is detached: check out a branch to push it.".into())),
            };
            validate_ref_name(&branch)?;
            let mut args = vec!["push"];
            if *force_with_lease {
                args.push("--force-with-lease");
            }
            if *set_upstream {
                args.extend(["-u", "origin", &branch]);
            } else if status.branch.as_deref() == Some(branch.as_str()) && status.upstream.is_some() {
                // The current branch with an upstream: git knows where it goes.
            } else {
                args.extend(["origin", &branch]);
            }
            outcome(repo, git_network(repo, &args)?, "Pushed.")
        }
        RepoOp::Checkout { name } => {
            validate_ref_name(name)?;
            if !local_branch_exists(repo, name) {
                return Err(TendrilError::Validation(format!("There is no local branch '{name}'.")));
            }
            outcome(repo, git_write(repo, &["checkout", "-q", name, "--"])?, "Switched branch.")
        }
        RepoOp::CheckoutRemote { remote_branch, local_name } => {
            validate_ref_name(remote_branch)?;
            let local = match local_name {
                Some(n) => n.clone(),
                None => remote_branch.split_once('/').map(|(_, rest)| rest.to_string()).unwrap_or_default(),
            };
            validate_ref_name(&local)?;
            if !list_refs(repo)?.iter().any(|r| r.kind == RefKind::Remote && &r.name == remote_branch) {
                return Err(TendrilError::Validation(format!("There is no remote branch '{remote_branch}'.")));
            }
            if local_branch_exists(repo, &local) {
                return Err(TendrilError::Validation(format!("A local branch '{local}' already exists: check it out instead.")));
            }
            outcome(
                repo,
                git_write(repo, &["checkout", "-q", "-b", &local, "--track", remote_branch])?,
                "Created a local branch and switched to it.",
            )
        }
        RepoOp::CheckoutCommit { hash } => {
            let hash = resolve_commit(repo, hash)?;
            outcome(repo, git_write(repo, &["checkout", "-q", "--detach", &hash])?, "Checked out that commit (detached).")
        }
        RepoOp::CreateBranch { name, from, checkout } => {
            validate_ref_name(name)?;
            if local_branch_exists(repo, name) {
                return Err(TendrilError::Validation(format!("A branch '{name}' already exists.")));
            }
            let start = match from {
                Some(f) => resolve_commit(repo, f)?,
                None => resolve_commit(repo, "HEAD")?,
            };
            let args: Vec<&str> =
                if *checkout { vec!["checkout", "-q", "-b", name, &start] } else { vec!["branch", name, &start] };
            outcome(repo, git_write(repo, &args)?, "Created the branch.")
        }
        RepoOp::RenameBranch { from, to } => {
            validate_ref_name(from)?;
            validate_ref_name(to)?;
            if !local_branch_exists(repo, from) {
                return Err(TendrilError::Validation(format!("There is no local branch '{from}'.")));
            }
            if local_branch_exists(repo, to) {
                return Err(TendrilError::Validation(format!("A branch '{to}' already exists.")));
            }
            outcome(repo, git_write(repo, &["branch", "-m", from, to])?, "Renamed the branch.")
        }
        RepoOp::DeleteBranch { name, force } => {
            validate_ref_name(name)?;
            if read_status(repo)?.branch.as_deref() == Some(name.as_str()) {
                return Err(TendrilError::Validation("That is the checked-out branch: switch to another one first.".into()));
            }
            if !local_branch_exists(repo, name) {
                return Err(TendrilError::Validation(format!("There is no local branch '{name}'.")));
            }
            let flag = if *force { "-D" } else { "-d" };
            outcome(repo, git_write(repo, &["branch", flag, name])?, "Deleted the branch.")
        }
        RepoOp::DeleteRemoteBranch { remote, name } => {
            validate_ref_name(remote)?;
            validate_ref_name(name)?;
            let remotes = git_read_ok(repo, &["remote"])?;
            if !remotes.lines().any(|r| r == remote) {
                return Err(TendrilError::Validation(format!("There is no remote '{remote}'.")));
            }
            outcome(repo, git_network(repo, &["push", remote, "--delete", name])?, "Deleted the remote branch.")
        }
        RepoOp::Stage { paths } => {
            paths_ok(paths)?;
            let mut args = vec!["add", "--"];
            args.extend(paths.iter().map(String::as_str));
            outcome(repo, git_write(repo, &args)?, "Staged.")
        }
        RepoOp::StageAll => outcome(repo, git_write(repo, &["add", "-A"])?, "Staged everything."),
        RepoOp::Unstage { paths } => {
            paths_ok(paths)?;
            let has_commits = resolve_commit(repo, "HEAD").is_ok();
            let mut args: Vec<&str> =
                if has_commits { vec!["restore", "--staged", "--"] } else { vec!["rm", "--cached", "-r", "-q", "--"] };
            args.extend(paths.iter().map(String::as_str));
            outcome(repo, git_write(repo, &args)?, "Unstaged.")
        }
        RepoOp::UnstageAll => {
            let args: &[&str] = if resolve_commit(repo, "HEAD").is_ok() {
                &["restore", "--staged", "."]
            } else {
                &["rm", "--cached", "-r", "-q", "."]
            };
            outcome(repo, git_write(repo, args)?, "Unstaged everything.")
        }
        RepoOp::Discard { paths } => {
            paths_ok(paths)?;
            let status = read_status(repo)?;
            for path in paths {
                let untracked = status.entries.iter().any(|e| e.untracked && &e.path == path);
                let out = if untracked {
                    git_write(repo, &["clean", "-f", "-q", "--", path])?
                } else {
                    git_write(repo, &["restore", "--", path])?
                };
                if !out.ok() {
                    return outcome(repo, out, "");
                }
            }
            outcome(repo, GitOut { code: 0, stdout: String::new(), stderr: String::new() }, "Discarded.")
        }
        RepoOp::Commit { message, amend } => {
            let message = message.trim();
            let status = read_status(repo)?;
            if message.is_empty() && !amend {
                return Err(TendrilError::Validation("A commit needs a message.".into()));
            }
            if !amend && status.staged() == 0 && status.state == RepoState::Clean {
                return Err(TendrilError::Validation("Nothing is staged to commit.".into()));
            }
            if status.conflicted() > 0 {
                return Err(TendrilError::Validation(
                    "Resolve the conflicts (and stage the files) before committing.".into(),
                ));
            }
            let mut args = vec!["-c", "core.editor=true", "commit", "-q"];
            if *amend {
                args.push("--amend");
            }
            if message.is_empty() {
                args.push("--no-edit");
            } else {
                args.extend(["-m", message]);
            }
            outcome(repo, git_write(repo, &args)?, if *amend { "Amended the last commit." } else { "Committed." })
        }
        RepoOp::Stash { message, include_untracked } => {
            let mut args = vec!["stash", "push", "-q"];
            if *include_untracked {
                args.push("-u");
            }
            if let Some(m) = message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                args.extend(["-m", m]);
            }
            outcome(repo, git_write(repo, &args)?, "Stashed your changes.")
        }
        RepoOp::StashPop { index } => outcome(
            repo,
            git_write(repo, &["stash", "pop", "-q", &format!("stash@{{{index}}}")])?,
            "Applied the stash and removed it.",
        ),
        RepoOp::StashApply { index } => {
            outcome(repo, git_write(repo, &["stash", "apply", "-q", &format!("stash@{{{index}}}")])?, "Applied the stash.")
        }
        RepoOp::StashDrop { index } => {
            outcome(repo, git_write(repo, &["stash", "drop", "-q", &format!("stash@{{{index}}}")])?, "Dropped the stash.")
        }
        RepoOp::Merge { branch, mode } => {
            validate_ref_name(branch)?;
            let target = resolve_commit(repo, branch)?;
            let flag = match mode {
                MergeMode::Default => None,
                MergeMode::NoFf => Some("--no-ff"),
                MergeMode::FfOnly => Some("--ff-only"),
            };
            let mut args = vec!["-c", "core.editor=true", "merge", "--no-edit"];
            args.extend(flag);
            args.push(&target);
            outcome(repo, git_write(repo, &args)?, "Merged.")
        }
        RepoOp::MergeAbort => {
            require_state(repo, &[RepoState::Merging], "merge")?;
            outcome(repo, git_write(repo, &["merge", "--abort"])?, "Aborted the merge.")
        }
        RepoOp::Rebase { onto } => {
            validate_ref_name(onto)?;
            let target = resolve_commit(repo, onto)?;
            outcome(repo, git_write(repo, &["-c", "core.editor=true", "rebase", &target])?, "Rebased.")
        }
        RepoOp::RebaseContinue => {
            require_state(repo, &[RepoState::Rebasing], "rebase")?;
            outcome(repo, git_write(repo, &["-c", "core.editor=true", "rebase", "--continue"])?, "Continued the rebase.")
        }
        RepoOp::RebaseSkip => {
            require_state(repo, &[RepoState::Rebasing], "rebase")?;
            outcome(repo, git_write(repo, &["-c", "core.editor=true", "rebase", "--skip"])?, "Skipped that commit.")
        }
        RepoOp::RebaseAbort => {
            require_state(repo, &[RepoState::Rebasing], "rebase")?;
            outcome(repo, git_write(repo, &["rebase", "--abort"])?, "Aborted the rebase.")
        }
        RepoOp::CherryPick { hash } => {
            let hash = resolve_commit(repo, hash)?;
            let mut args = vec!["-c", "core.editor=true", "cherry-pick"];
            if is_merge_commit(repo, &hash) {
                args.extend(["-m", "1"]);
            }
            args.push(&hash);
            outcome(repo, git_write(repo, &args)?, "Cherry-picked.")
        }
        RepoOp::CherryPickContinue => {
            require_state(repo, &[RepoState::CherryPicking], "cherry-pick")?;
            outcome(
                repo,
                git_write(repo, &["-c", "core.editor=true", "cherry-pick", "--continue"])?,
                "Continued the cherry-pick.",
            )
        }
        RepoOp::CherryPickAbort => {
            require_state(repo, &[RepoState::CherryPicking], "cherry-pick")?;
            outcome(repo, git_write(repo, &["cherry-pick", "--abort"])?, "Aborted the cherry-pick.")
        }
        RepoOp::Revert { hash } => {
            let hash = resolve_commit(repo, hash)?;
            let mut args = vec!["-c", "core.editor=true", "revert", "--no-edit"];
            if is_merge_commit(repo, &hash) {
                args.extend(["-m", "1"]);
            }
            args.push(&hash);
            outcome(repo, git_write(repo, &args)?, "Reverted.")
        }
        RepoOp::RevertContinue => {
            require_state(repo, &[RepoState::Reverting], "revert")?;
            outcome(repo, git_write(repo, &["-c", "core.editor=true", "revert", "--continue"])?, "Continued the revert.")
        }
        RepoOp::RevertAbort => {
            require_state(repo, &[RepoState::Reverting], "revert")?;
            outcome(repo, git_write(repo, &["revert", "--abort"])?, "Aborted the revert.")
        }
        RepoOp::CreateTag { name, at, message } => {
            validate_ref_name(name)?;
            let target = match at {
                Some(a) => resolve_commit(repo, a)?,
                None => resolve_commit(repo, "HEAD")?,
            };
            let mut args = vec!["tag"];
            if let Some(m) = message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                args.extend(["-a", "-m", m]);
            }
            args.extend([name.as_str(), target.as_str()]);
            outcome(repo, git_write(repo, &args)?, "Created the tag.")
        }
        RepoOp::DeleteTag { name } => {
            validate_ref_name(name)?;
            outcome(repo, git_write(repo, &["tag", "-d", name])?, "Deleted the tag.")
        }
        RepoOp::Reset { to, mode } => {
            let target = resolve_commit(repo, to)?;
            let flag = match mode {
                ResetMode::Soft => "--soft",
                ResetMode::Mixed => "--mixed",
                ResetMode::Hard => "--hard",
            };
            outcome(repo, git_write(repo, &["reset", "-q", flag, &target])?, "Moved the branch.")
        }
        RepoOp::WorktreeAdd { path, branch, create } => {
            validate_ref_name(branch)?;
            let p = Path::new(path);
            if !p.is_absolute() || path.contains('\0') || p.exists() {
                return Err(TendrilError::Validation(
                    "A new worktree needs an absolute path that does not exist yet.".into(),
                ));
            }
            if p.parent().map(|d| !d.is_dir()).unwrap_or(true) {
                return Err(TendrilError::Validation("The folder to put the worktree in does not exist.".into()));
            }
            let args: Vec<&str> =
                if *create { vec!["worktree", "add", "-b", branch, path] } else { vec!["worktree", "add", path, branch] };
            outcome(repo, git_write(repo, &args)?, "Added the worktree.")
        }
        RepoOp::WorktreeRemove { path, force } => {
            // Compared as real paths: git reports the canonical one, which differs from what a client
            // typed wherever a folder is reached through a symlink (macOS's /var is /private/var).
            let want = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
            let known = list_worktrees(repo)?
                .into_iter()
                .find(|w| !w.main && std::fs::canonicalize(&w.path).unwrap_or_else(|_| (&w.path).into()) == want);
            if known.is_none() {
                return Err(TendrilError::Validation("That is not one of this repository's added worktrees.".into()));
            }
            let mut args = vec!["worktree", "remove"];
            if *force {
                args.push("--force");
            }
            args.push(path);
            outcome(repo, git_write(repo, &args)?, "Removed the worktree.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::Scratch;
    use super::*;

    fn op(repo: &Scratch, op: RepoOp) -> OpOutcome {
        run_op(&repo.dir, &op).unwrap_or_else(|e| panic!("{op:?} was refused: {e}"))
    }

    fn refused(repo: &Scratch, o: RepoOp) -> String {
        run_op(&repo.dir, &o).expect_err("should be refused").to_string()
    }

    fn branch(repo: &Scratch) -> Option<String> {
        read_status(&repo.dir).unwrap().branch
    }

    fn last_subject(repo: &Scratch) -> String {
        super::super::read_summary(&repo.dir).unwrap().last_commit.unwrap().subject
    }

    #[test]
    fn stage_commit_amend_unstage_and_discard() {
        let repo = Scratch::new();
        repo.commit("a.txt", "one\n", "first");
        repo.write("a.txt", "two\n");
        repo.write("new.txt", "new\n");

        assert!(refused(&repo, RepoOp::Commit { message: "x".into(), amend: false }).contains("Nothing is staged"));
        assert!(op(&repo, RepoOp::Stage { paths: vec!["a.txt".into()] }).ok);
        assert_eq!(read_status(&repo.dir).unwrap().staged(), 1);
        assert!(op(&repo, RepoOp::Unstage { paths: vec!["a.txt".into()] }).ok);
        assert_eq!(read_status(&repo.dir).unwrap().staged(), 0);

        assert!(op(&repo, RepoOp::StageAll).ok);
        assert!(refused(&repo, RepoOp::Commit { message: "  ".into(), amend: false }).contains("needs a message"));
        assert!(op(&repo, RepoOp::Commit { message: "second\n\nwith a body".into(), amend: false }).ok);
        assert!(!read_status(&repo.dir).unwrap().is_dirty());

        assert!(op(&repo, RepoOp::Commit { message: "second, reworded".into(), amend: true }).ok);
        assert_eq!(last_subject(&repo), "second, reworded");

        repo.write("a.txt", "scrap\n");
        repo.write("junk.txt", "junk\n");
        assert!(op(&repo, RepoOp::Discard { paths: vec!["a.txt".into(), "junk.txt".into()] }).ok);
        assert_eq!(std::fs::read_to_string(repo.dir.join("a.txt")).unwrap(), "two\n", "a tracked file goes back to what it was");
        assert!(!repo.dir.join("junk.txt").exists(), "an untracked file is removed");
    }

    #[test]
    fn branches_are_created_switched_renamed_and_deleted_safely() {
        let repo = Scratch::new();
        repo.commit("a.txt", "one\n", "first");

        assert!(op(&repo, RepoOp::CreateBranch { name: "feature/x".into(), from: None, checkout: true }).ok);
        assert_eq!(branch(&repo).as_deref(), Some("feature/x"));
        repo.commit("f.txt", "f\n", "feature work");
        assert!(op(&repo, RepoOp::Checkout { name: "main".into() }).ok);

        assert!(refused(&repo, RepoOp::DeleteBranch { name: "main".into(), force: false }).contains("checked-out"));
        let unmerged = op(&repo, RepoOp::DeleteBranch { name: "feature/x".into(), force: false });
        assert!(!unmerged.ok, "git refuses an unmerged branch and says why");
        assert!(unmerged.message.contains("not fully merged"), "{}", unmerged.message);
        assert!(op(&repo, RepoOp::RenameBranch { from: "feature/x".into(), to: "feature/y".into() }).ok);
        assert!(op(&repo, RepoOp::DeleteBranch { name: "feature/y".into(), force: true }).ok);
        assert!(!list_refs(&repo.dir).unwrap().iter().any(|r| r.name.starts_with("feature/")));

        assert!(refused(&repo, RepoOp::CreateBranch { name: "main".into(), from: None, checkout: false }).contains("already exists"));
        assert!(refused(&repo, RepoOp::Checkout { name: "ghost".into() }).contains("no local branch"));
    }

    #[test]
    fn nothing_a_client_sends_can_become_a_git_option_or_escape_the_repository() {
        let repo = Scratch::new();
        repo.commit("a.txt", "one\n", "first");
        for bad in ["--all", "-D", "--output=/tmp/x", "a b", "x..y"] {
            assert!(run_op(&repo.dir, &RepoOp::CreateBranch { name: bad.into(), from: None, checkout: false }).is_err(), "{bad}");
            assert!(run_op(&repo.dir, &RepoOp::Merge { branch: bad.into(), mode: MergeMode::Default }).is_err(), "{bad}");
            assert!(run_op(&repo.dir, &RepoOp::Reset { to: bad.into(), mode: ResetMode::Hard }).is_err(), "{bad}");
            assert!(run_op(&repo.dir, &RepoOp::CherryPick { hash: bad.into() }).is_err(), "{bad}");
        }
        for bad in ["../x", "/etc/hosts", "a/../../b"] {
            assert!(run_op(&repo.dir, &RepoOp::Stage { paths: vec![bad.into()] }).is_err(), "{bad}");
            assert!(run_op(&repo.dir, &RepoOp::Discard { paths: vec![bad.into()] }).is_err(), "{bad}");
        }
        assert!(run_op(&repo.dir, &RepoOp::Stage { paths: vec![] }).is_err());
    }

    #[test]
    fn a_clean_merge_works_and_a_conflicting_one_reports_the_files_and_can_be_aborted() {
        let repo = Scratch::new();
        repo.commit("a.txt", "base\n", "base");
        repo.git(&["checkout", "-q", "-b", "other"]);
        repo.commit("a.txt", "from other\n", "other edits a");
        repo.git(&["checkout", "-q", "main"]);

        // A fast-forward-only merge of a diverged branch is refused with git's reason...
        repo.commit("b.txt", "main only\n", "main adds b");
        let no_ff = op(&repo, RepoOp::Merge { branch: "other".into(), mode: MergeMode::FfOnly });
        assert!(!no_ff.ok);

        // ...and a plain merge of a branch that edits what main edited stops on the conflict.
        repo.commit("a.txt", "from main\n", "main edits a");
        let conflict = op(&repo, RepoOp::Merge { branch: "other".into(), mode: MergeMode::Default });
        assert!(!conflict.ok);
        assert_eq!(conflict.conflicts, vec!["a.txt".to_string()]);
        assert_eq!(conflict.state, RepoState::Merging);
        assert!(conflict.message.contains("conflicts in 1 file"));
        assert!(refused(&repo, RepoOp::Commit { message: "x".into(), amend: false }).contains("conflicts"));

        assert!(op(&repo, RepoOp::MergeAbort).ok);
        assert_eq!(read_status(&repo.dir).unwrap().state, RepoState::Clean);
        assert!(refused(&repo, RepoOp::MergeAbort).contains("no merge in progress"));

        // A branch that touches different files merges cleanly, as a merge commit when asked.
        repo.git(&["checkout", "-q", "-b", "side", "HEAD~1"]);
        repo.commit("side.txt", "s\n", "side file");
        repo.git(&["checkout", "-q", "main"]);
        assert!(op(&repo, RepoOp::Merge { branch: "side".into(), mode: MergeMode::NoFf }).ok);
        assert_eq!(super::super::read_commit(&repo.dir, "HEAD").unwrap().parents.len(), 2);
    }

    #[test]
    fn stash_tag_reset_cherry_pick_and_revert() {
        let repo = Scratch::new();
        let first = repo.commit("a.txt", "one\n", "first");
        repo.write("a.txt", "wip\n");
        assert!(op(&repo, RepoOp::Stash { message: Some("parked".into()), include_untracked: false }).ok);
        assert!(!read_status(&repo.dir).unwrap().is_dirty());
        assert_eq!(super::super::list_stashes(&repo.dir).unwrap().len(), 1);
        assert!(op(&repo, RepoOp::StashPop { index: 0 }).ok);
        assert_eq!(std::fs::read_to_string(repo.dir.join("a.txt")).unwrap(), "wip\n");
        assert!(op(&repo, RepoOp::Discard { paths: vec!["a.txt".into()] }).ok);

        assert!(op(&repo, RepoOp::CreateTag { name: "v1".into(), at: None, message: Some("one".into()) }).ok);
        let second = repo.commit("b.txt", "two\n", "second");
        assert!(op(&repo, RepoOp::Revert { hash: second.clone() }).ok);
        assert!(!repo.dir.join("b.txt").exists(), "a revert undoes the commit with a new one");
        assert!(op(&repo, RepoOp::Reset { to: second.clone(), mode: ResetMode::Hard }).ok);
        assert!(repo.dir.join("b.txt").exists());

        repo.git(&["checkout", "-q", "-b", "pick-source", &first]);
        let picked = repo.commit("c.txt", "three\n", "third on another branch");
        assert!(op(&repo, RepoOp::Checkout { name: "main".into() }).ok);
        assert!(op(&repo, RepoOp::CherryPick { hash: picked }).ok);
        assert!(repo.dir.join("c.txt").exists());
        assert!(op(&repo, RepoOp::DeleteTag { name: "v1".into() }).ok);
    }

    #[test]
    fn rebase_works_and_a_conflicting_rebase_can_be_aborted() {
        let repo = Scratch::new();
        repo.commit("a.txt", "base\n", "base");
        repo.git(&["checkout", "-q", "-b", "topic"]);
        repo.commit("a.txt", "topic\n", "topic edits a");
        repo.git(&["checkout", "-q", "main"]);
        repo.commit("a.txt", "main\n", "main edits a");
        repo.git(&["checkout", "-q", "topic"]);

        let stopped = op(&repo, RepoOp::Rebase { onto: "main".into() });
        assert!(!stopped.ok);
        assert_eq!(stopped.state, RepoState::Rebasing);
        assert_eq!(stopped.conflicts, vec!["a.txt".to_string()]);
        assert!(op(&repo, RepoOp::RebaseAbort).ok);
        assert_eq!(read_status(&repo.dir).unwrap().state, RepoState::Clean);
        assert_eq!(branch(&repo).as_deref(), Some("topic"));
    }

    #[test]
    fn fetch_pull_and_push_talk_to_a_remote_and_refuse_unsafe_pulls() {
        let remote = std::env::temp_dir().join(format!("tendril-remote-{}.git", uuid::Uuid::new_v4().simple()));
        let repo = Scratch::new();
        repo.git(&["init", "-q", "--bare", remote.to_str().unwrap()]);
        repo.commit("a.txt", "one\n", "first");
        repo.git(&["remote", "add", "origin", remote.to_str().unwrap()]);

        // No upstream yet: pushing needs it set, and does so.
        assert!(op(&repo, RepoOp::Push { branch: None, set_upstream: true, force_with_lease: false }).ok);
        assert_eq!(read_status(&repo.dir).unwrap().upstream.as_deref(), Some("origin/main"));

        // A second clone pushes a commit, so ours is behind.
        let other = Scratch::new();
        other.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        other.git(&["fetch", "-q", "origin"]);
        other.git(&["checkout", "-q", "-b", "main2", "--track", "origin/main"]);
        other.commit("b.txt", "from other\n", "other commit");
        other.git(&["push", "-q", "origin", "HEAD:main"]);

        assert!(op(&repo, RepoOp::Fetch { prune: true }).ok);
        assert_eq!(read_status(&repo.dir).unwrap().behind, 1);
        assert!(op(&repo, RepoOp::Pull { mode: PullMode::FfOnly }).ok);
        assert!(repo.dir.join("b.txt").exists());

        // Diverge: a local commit and a remote one. A fast-forward-only pull must refuse, not merge.
        repo.commit("local.txt", "l\n", "local commit");
        other.commit("remote.txt", "r\n", "another remote commit");
        other.git(&["push", "-q", "origin", "HEAD:main"]);
        op(&repo, RepoOp::Fetch { prune: false });
        let refused_pull = op(&repo, RepoOp::Pull { mode: PullMode::FfOnly });
        assert!(!refused_pull.ok, "{}", refused_pull.message);
        assert!(op(&repo, RepoOp::Pull { mode: PullMode::Rebase }).ok);
        assert!(op(&repo, RepoOp::Push { branch: None, set_upstream: false, force_with_lease: false }).ok);

        // A remote branch can be created, checked out locally, and deleted.
        repo.git(&["branch", "pushed-branch"]);
        assert!(op(&repo, RepoOp::Push { branch: Some("pushed-branch".into()), set_upstream: false, force_with_lease: false }).ok);
        op(&repo, RepoOp::Fetch { prune: false });
        assert!(op(&repo, RepoOp::DeleteBranch { name: "pushed-branch".into(), force: true }).ok);
        assert!(op(&repo, RepoOp::CheckoutRemote { remote_branch: "origin/pushed-branch".into(), local_name: None }).ok);
        assert_eq!(branch(&repo).as_deref(), Some("pushed-branch"));
        assert!(op(&repo, RepoOp::Checkout { name: "main".into() }).ok);
        assert!(op(&repo, RepoOp::DeleteRemoteBranch { remote: "origin".into(), name: "pushed-branch".into() }).ok);
        assert!(refused(&repo, RepoOp::DeleteRemoteBranch { remote: "nowhere".into(), name: "x".into() }).contains("no remote"));
        let _ = std::fs::remove_dir_all(remote);
    }

    #[test]
    fn worktrees_can_be_added_and_only_added_ones_removed() {
        let repo = Scratch::new();
        repo.commit("a.txt", "one\n", "first");
        let wt = std::env::temp_dir().join(format!("tendril-wt-{}", uuid::Uuid::new_v4().simple()));
        assert!(refused(&repo, RepoOp::WorktreeAdd { path: "relative/path".into(), branch: "wt".into(), create: true }).contains("absolute"));
        assert!(op(&repo, RepoOp::WorktreeAdd { path: wt.to_string_lossy().to_string(), branch: "wt".into(), create: true }).ok);
        assert_eq!(list_worktrees(&repo.dir).unwrap().len(), 2);
        assert!(refused(&repo, RepoOp::WorktreeRemove { path: repo.dir.to_string_lossy().to_string(), force: true }).contains("not one of"));
        assert!(op(&repo, RepoOp::WorktreeRemove { path: wt.to_string_lossy().to_string(), force: false }).ok);
        assert_eq!(list_worktrees(&repo.dir).unwrap().len(), 1);
    }

    #[test]
    fn operations_round_trip_through_json_the_way_the_page_sends_them() {
        let o: RepoOp = serde_json::from_str(r#"{"op":"merge","branch":"feature/x","mode":"noFf"}"#).unwrap();
        assert_eq!(o, RepoOp::Merge { branch: "feature/x".into(), mode: MergeMode::NoFf });
        let o: RepoOp = serde_json::from_str(r#"{"op":"pull"}"#).unwrap();
        assert_eq!(o, RepoOp::Pull { mode: PullMode::FfOnly });
        let o: RepoOp = serde_json::from_str(r#"{"op":"push","setUpstream":true}"#).unwrap();
        assert_eq!(o, RepoOp::Push { branch: None, set_upstream: true, force_with_lease: false });
        assert!(serde_json::from_str::<RepoOp>(r#"{"op":"rm-rf"}"#).is_err());
    }
}
