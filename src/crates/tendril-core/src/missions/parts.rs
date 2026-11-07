//! Stacked missions: every milestone ships as its own small pull request, stacked on the one before.
//!
//! Workers still commit on the one shared mission branch, so nothing about how a milestone runs
//! changes. When a milestone passes, the commits it added (`from..to` on the mission branch) become
//! a part: a branch of its own cut from the previous part's branch (the repo's base branch for the
//! first), recorded on the milestone under `parts`. The driver then pushes it and opens its pull
//! request against that branch, with a "Part k of n" section listing every part, so the whole
//! mission can be reviewed in order while later milestones are still running.
//!
//! When a part merges, the parts above it move: the next one is replayed onto the repo's base branch
//! with `git rebase --onto <base> <recorded base commit>` (right whether the part was squashed,
//! rebased or merged), force-pushed with a lease and retargeted; each one after it is replayed onto
//! the rewritten branch below it. A conflict is left for a person, with the rebase aborted. This is
//! the same model as [`crate::plans::stack`], kept on the mission because a milestone's part has no
//! worktree of its own: every rebase runs in a short-lived detached checkout.
//!
//! A mission without `stacked` (one created before this existed) records no parts and keeps its
//! single pull request at the end.

use crate::git::service::run_git;
use crate::missions::model::{MilestonePart, MilestoneState, Milestone, MissionYaml};
use crate::pull_request::{GitHub, NewPr};
use std::path::Path;
use std::process::Command;

/// The milestones that ship as parts, in order: every milestone not dropped, except one that passed
/// without changing anything.
pub fn series(mission: &MissionYaml) -> Vec<&Milestone> {
    mission
        .milestones
        .iter()
        .filter(|ms| ms.state != MilestoneState::Skipped)
        .filter(|ms| ms.state != MilestoneState::Passed || !ms.parts.is_empty())
        .collect()
}

/// `(k, n)`: where milestone `id` sits in the series.
pub fn position(mission: &MissionYaml, id: &str) -> Option<(usize, usize)> {
    let series = series(mission);
    let k = series.iter().position(|ms| ms.id.eq_ignore_ascii_case(id))? + 1;
    Some((k, series.len()))
}

/// `[00012] Add SSO (part 2 of 4): Session store`.
pub fn pr_title(mission: &MissionYaml, mission_id: &str, ms: &Milestone) -> String {
    match position(mission, &ms.id) {
        Some((k, n)) => format!("[{mission_id}] {} (part {k} of {n}): {}", mission.title, ms.title),
        None => format!("[{mission_id}] {}: {}", mission.title, ms.title),
    }
}

/// The "Part k of n" section that opens every part's pull request body. `repo` picks which of a
/// milestone's pull requests to link when it changed more than one repo.
pub fn series_section(mission: &MissionYaml, id: &str, repo: &str) -> Option<String> {
    let (k, n) = position(mission, id)?;
    let mut out = format!(
        "### Part {k} of {n}: {}\n\nOne part of a stacked series: each pull request shows only its own milestone. Review and merge them in order; the later parts are moved onto the base branch as each one merges.\n\n",
        mission.title
    );
    for (i, ms) in series(mission).into_iter().enumerate() {
        let line = if i + 1 == k {
            format!("{}. **[{}] {}** (this PR)", i + 1, ms.id, ms.title)
        } else {
            let pr = ms
                .parts
                .get(repo)
                .and_then(|p| p.pr.clone())
                .or_else(|| ms.parts.values().find_map(|p| p.pr.clone()));
            match pr {
                Some(url) => format!("{}. [{}] {} - {url}", i + 1, ms.id, ms.title),
                None => format!("{}. [{}] {} - not opened yet", i + 1, ms.id, ms.title),
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

/// The rest of a part's body: what the milestone delivered and what it was judged against.
pub fn pr_body(mission: &MissionYaml, ms: &Milestone, repo: &str) -> String {
    let mut body = series_section(mission, &ms.id, repo).map(|s| format!("{s}\n\n")).unwrap_or_default();
    body.push_str(&format!("## {} — {}\n\n", ms.id, ms.title));
    match ms.summary.as_ref().filter(|s| !s.trim().is_empty()) {
        Some(summary) => body.push_str(&format!("{}\n\n", summary.trim())),
        None if !ms.objective.is_empty() => body.push_str(&format!("{}\n\n", ms.objective)),
        None => {}
    }
    if !ms.acceptance.is_empty() {
        body.push_str("**Acceptance**\n\n");
        for a in &ms.acceptance {
            body.push_str(&format!("- [x] {a}\n"));
        }
        body.push('\n');
    }
    body.push_str(&format!("Goal of the mission: {}\n", mission.goal.lines().next().unwrap_or_default()));
    body
}

/// The milestone whose part in `repo` should be opened next, if any: the first passed one without a
/// pull request. Parts open strictly in order, because each one's base is the part before it.
pub fn next_to_open<'a>(mission: &'a MissionYaml, repo: &str) -> Option<&'a Milestone> {
    mission
        .milestones
        .iter()
        .filter(|ms| ms.state == MilestoneState::Passed)
        .filter(|ms| ms.parts.get(repo).is_some_and(|p| !p.merged))
        .find(|ms| ms.parts.get(repo).is_some_and(|p| p.pr.is_none()))
}

/// The part below milestone `id` in `repo` that is still open: what its part builds on. `None`
/// means it builds on the repo's base branch.
pub fn part_below<'a>(mission: &'a MissionYaml, id: &str, repo: &str) -> Option<&'a MilestonePart> {
    mission
        .milestones
        .iter()
        .take_while(|ms| !ms.id.eq_ignore_ascii_case(id))
        .filter(|ms| ms.state == MilestoneState::Passed)
        .filter_map(|ms| ms.parts.get(repo))
        .filter(|p| !p.merged && p.pr.is_some())
        .last()
}

/// Every repo any milestone shipped a part in.
pub fn repos(mission: &MissionYaml) -> Vec<String> {
    let mut out: Vec<String> = mission.milestones.iter().flat_map(|ms| ms.parts.keys().cloned()).collect();
    out.sort();
    out.dedup();
    out
}

/// Some part has an open pull request: the stack still needs watching, even on a finished mission.
pub fn has_open_parts(mission: &MissionYaml) -> bool {
    mission
        .milestones
        .iter()
        .flat_map(|ms| ms.parts.values())
        .any(|p| p.pr.is_some() && !p.merged)
}

/// Every part that was opened has merged, and there was at least one.
pub fn all_merged(mission: &MissionYaml) -> bool {
    let opened: Vec<&MilestonePart> = mission
        .milestones
        .iter()
        .flat_map(|ms| ms.parts.values())
        .filter(|p| p.pr.is_some())
        .collect();
    !opened.is_empty() && opened.iter().all(|p| p.merged)
}

/// One part to move after a part below it merged (or was moved itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restack {
    pub milestone: String,
    /// The branch it now builds on: the part below that is still open, or the repo's base branch.
    pub new_base: String,
    /// Moving onto the repo's base branch rather than onto another part.
    pub onto_base_branch: bool,
}

/// What has to move in `repo`, lowest part first. A part whose base is no longer the open part below
/// it moves onto that part (or onto the base branch); every published part above a moved one is
/// replayed too, because the branch it sits on was rewritten. The driver stops at the first part that
/// fails to move; nothing above it can be replayed onto a branch that is not where it should be.
pub fn restack_plan(mission: &MissionYaml, repo: &str) -> Vec<Restack> {
    let mut steps = Vec::new();
    let mut below: Option<&MilestonePart> = None;
    let mut moved_below = false;
    for ms in mission.milestones.iter().filter(|ms| ms.state == MilestoneState::Passed) {
        let Some(part) = ms.parts.get(repo) else { continue };
        if part.merged || part.pr.is_none() {
            continue;
        }
        let (new_base, onto_base_branch) = match below {
            Some(p) => (p.branch.clone(), false),
            None => (part.onto.clone(), true),
        };
        // A part whose last move failed is tried again: the branch below it may already be rewritten.
        if part.base != new_base || moved_below || part.error.is_some() {
            steps.push(Restack { milestone: ms.id.clone(), new_base, onto_base_branch });
            moved_below = true;
        }
        below = Some(part);
    }
    steps
}

// -------------------------------------------------------------------------------------------------
// Git and GitHub. Everything below blocks; the driver runs it off the async runtime.
// -------------------------------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> std::result::Result<String, String> {
    match run_git(args, dir) {
        Ok((0, out, _)) => Ok(out.trim().to_string()),
        Ok((_, _, err)) => Err(err.trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

fn rev(repo: &Path, reference: &str) -> Option<String> {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")])
        .ok()
        .filter(|s| !s.is_empty())
}

fn is_ancestor(repo: &Path, ancestor: &str, of: &str) -> bool {
    matches!(run_git(&["merge-base", "--is-ancestor", ancestor, of], repo), Ok((0, _, _)))
}

/// The repo's base branch: the configured one, else the remote's HEAD, else `main`.
pub fn base_branch(repo: &Path, configured: Option<&str>) -> String {
    if let Some(b) = configured.map(str::trim).filter(|b| !b.is_empty()) {
        return b.to_string();
    }
    git(repo, &["symbolic-ref", "refs/remotes/origin/HEAD"])
        .ok()
        .map(|r| r.replace("refs/remotes/origin/", ""))
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".to_string())
}

/// Puts `branch` at the commits `from..tip` replayed onto `onto`, and returns its new tip. When
/// `tip` already sits on `onto` the branch simply points at it; otherwise the commits are rebased in
/// a detached checkout at `scratch`, which is removed again whatever happens.
pub fn replay(repo: &Path, branch: &str, from: &str, tip: &str, onto: &str, scratch: &Path) -> std::result::Result<String, String> {
    let onto_sha = rev(repo, onto).ok_or_else(|| format!("{onto} does not exist"))?;
    let tip_sha = rev(repo, tip).ok_or_else(|| format!("{tip} does not exist"))?;
    let new_tip = if is_ancestor(repo, &onto_sha, &tip_sha) {
        tip_sha
    } else {
        let _ = git(repo, &["worktree", "remove", "--force", &scratch.to_string_lossy()]);
        let _ = std::fs::remove_dir_all(scratch);
        let _ = git(repo, &["worktree", "prune"]);
        if let Some(parent) = scratch.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        git(repo, &["worktree", "add", "--detach", &scratch.to_string_lossy(), &tip_sha])?;
        let rebased = git(scratch, &["rebase", "--onto", &onto_sha, from]).and_then(|_| git(scratch, &["rev-parse", "HEAD"]));
        if rebased.is_err() {
            let _ = git(scratch, &["rebase", "--abort"]);
        }
        let _ = git(repo, &["worktree", "remove", "--force", &scratch.to_string_lossy()]);
        let _ = std::fs::remove_dir_all(scratch);
        let _ = git(repo, &["worktree", "prune"]);
        rebased.map_err(|e| format!("conflicts replaying it onto {onto}; rebase it by hand ({e})"))?
    };
    git(repo, &["branch", "-f", branch, &new_tip])?;
    Ok(new_tip)
}

/// Brings the local `branch` up to its remote copy when someone pushed to the pull request (a fix
/// for review or CI), so a restack never force-pushes their commits away. Diverged is an error.
fn follow_remote(repo: &Path, branch: &str) -> std::result::Result<(), String> {
    let remote = format!("origin/{branch}");
    let (Some(local), Some(theirs)) = (rev(repo, branch), rev(repo, &remote)) else {
        return Ok(());
    };
    if local == theirs || is_ancestor(repo, &theirs, &local) {
        return Ok(());
    }
    if is_ancestor(repo, &local, &theirs) {
        return git(repo, &["branch", "-f", branch, &theirs]).map(|_| ());
    }
    Err(format!("{branch} has commits on GitHub that are not here and the other way round; move it by hand"))
}

fn push(repo: &Path, branch: &str, lease: Option<&str>, skip_hooks: bool) -> std::result::Result<(), String> {
    let lease_arg = lease.map(|sha| format!("--force-with-lease={branch}:{sha}"));
    let mut args = vec!["push".to_string()];
    if let Some(l) = &lease_arg {
        args.push(l.clone());
    }
    if skip_hooks {
        args.push("--no-verify".into());
    }
    args.extend(["-u".into(), "origin".into(), branch.to_string()]);
    let mut cmd = Command::new("git");
    cmd.args(&args).current_dir(repo);
    if skip_hooks {
        cmd.env("HUSKY", "0");
    }
    let out = cmd.output().map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// How opening one part went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// Pushed and its pull request is open; the part as it now stands.
    Opened(MilestonePart),
    /// The milestone changed nothing on top of the part below: there is nothing to ship.
    Empty,
    /// It could not be opened now; why.
    Failed(String),
}

/// Cuts, pushes and opens one part. `below` is the open part it builds on (`None`: the base branch).
#[allow(clippy::too_many_arguments)]
pub fn open_part(
    gh: &dyn GitHub,
    repo: &Path,
    part: &MilestonePart,
    below: Option<&MilestonePart>,
    onto: &str,
    title: &str,
    body: &str,
    scratch: &Path,
    skip_hooks: bool,
) -> Opened {
    let remote = git(repo, &["remote", "get-url", "origin"]).unwrap_or_default();
    let Some(slug) = gh.repo_slug(&remote) else {
        return Opened::Failed(format!("{} has no GitHub origin, so its part cannot be opened", repo.display()));
    };
    let _ = git(repo, &["fetch", "origin", "--prune"]);
    let (base, onto_ref) = match below {
        Some(p) => {
            if let Err(e) = follow_remote(repo, &p.branch) {
                return Opened::Failed(e);
            }
            (p.branch.clone(), p.branch.clone())
        }
        None => {
            // The lowest open part keeps the base it was written on while the base branch still has
            // it (GitHub diffs against the merge base); after a squash merge below it, it moves.
            let remote = format!("origin/{onto}");
            let keep = is_ancestor(repo, &part.from, &remote);
            (onto.to_string(), if keep { part.from.clone() } else { remote })
        }
    };
    let Some(base_commit) = rev(repo, &onto_ref) else {
        return Opened::Failed(format!("{onto_ref} does not exist in {}", repo.display()));
    };
    let tip = match replay(repo, &part.branch, &part.from, &part.to, &onto_ref, scratch) {
        Ok(tip) => tip,
        Err(e) => return Opened::Failed(e),
    };
    let range = format!("{base_commit}..{tip}");
    let ahead = git(repo, &["rev-list", "--count", &range]).ok().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    if ahead == 0 {
        return Opened::Empty;
    }

    if let Err(e) = push(repo, &part.branch, None, skip_hooks) {
        // The branch is the mission's alone: a stale copy from an earlier attempt loses.
        let theirs = rev(repo, &format!("origin/{}", part.branch)).unwrap_or_default();
        if let Err(e2) = push(repo, &part.branch, Some(&theirs), skip_hooks) {
            return Opened::Failed(format!("push failed: {e} / {e2}"));
        }
    }
    let commits = git(repo, &["log", "--reverse", "--format=- %h %s", &range]).unwrap_or_default();
    let full_body = format!("{}\n\n---\n{}\n\n---\nCreated using [Ivy Tendril](https://ivy.app).", body.trim(), commits.trim());
    let pr = match gh.open_pr_for_branch(&slug, &part.branch) {
        Ok(Some(existing)) => existing,
        Ok(None) => {
            let created = gh.create_pr(&NewPr {
                repo: &slug,
                base: &base,
                head: &part.branch,
                title,
                body: &full_body,
                draft: false,
                reviewers: &[],
            });
            match created {
                Ok(pr) => pr,
                Err(e) => return Opened::Failed(e.to_string()),
            }
        }
        Err(e) => return Opened::Failed(e.to_string()),
    };
    if let Some(number) = below.and_then(|p| p.pr.as_deref()).and_then(|u| u.rsplit('/').next()).and_then(|n| n.parse::<u64>().ok()) {
        let _ = gh.comment(&slug, number, &format!("Next part: {} (built on this PR's branch).", pr.url));
    }
    Opened::Opened(MilestonePart {
        base,
        base_commit,
        onto: onto.to_string(),
        pr: Some(pr.url),
        error: None,
        ..part.clone()
    })
}

/// Points a pull request at another base branch with `gh`.
pub fn gh_retarget(url: &str, base: &str) -> std::result::Result<(), String> {
    let out = Command::new("gh").args(["pr", "edit", url, "--base", base]).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Moves one part onto `new_base` (another part's branch, or `onto` on the remote), pushes it with a
/// lease and, when the base changed, retargets its pull request with `retarget(url, base)`. Returns
/// the part as it now stands.
pub fn move_part(
    repo: &Path,
    part: &MilestonePart,
    step: &Restack,
    scratch: &Path,
    skip_hooks: bool,
    retarget: &dyn Fn(&str, &str) -> std::result::Result<(), String>,
) -> std::result::Result<MilestonePart, String> {
    let _ = git(repo, &["fetch", "origin", "--prune"]);
    follow_remote(repo, &part.branch)?;
    let onto_ref = if step.onto_base_branch { format!("origin/{}", step.new_base) } else { step.new_base.clone() };
    let new_base_commit = rev(repo, &onto_ref).ok_or_else(|| format!("{onto_ref} does not exist"))?;
    let lease = rev(repo, &format!("origin/{}", part.branch)).unwrap_or_default();
    replay(repo, &part.branch, &part.base_commit, &part.branch, &onto_ref, scratch)?;
    push(repo, &part.branch, Some(&lease), skip_hooks).map_err(|e| format!("moved onto {onto_ref} but the push failed: {e}"))?;
    if part.base != step.new_base {
        if let Some(url) = &part.pr {
            retarget(url, &step.new_base)
                .map_err(|e| format!("moved onto {onto_ref} but retargeting its pull request failed: {e}"))?;
        }
    }
    Ok(MilestonePart { base: step.new_base.clone(), base_commit: new_base_commit, error: None, ..part.clone() })
}

/// Lands what reached the mission branch after the last part (a quick fix in Review) on that part,
/// and pushes it. Returns the part as it now stands, or `None` when there was nothing to add.
pub fn extend_last_part(repo: &Path, part: &MilestonePart, mission_tip: &str, scratch: &Path, skip_hooks: bool) -> std::result::Result<Option<MilestonePart>, String> {
    if part.to == mission_tip || !is_ancestor(repo, &part.to, mission_tip) {
        return Ok(None);
    }
    let _ = git(repo, &["fetch", "origin", "--prune"]);
    follow_remote(repo, &part.branch)?;
    replay(repo, &part.branch, &part.to, mission_tip, &part.branch, scratch)?;
    push(repo, &part.branch, None, skip_hooks)?;
    Ok(Some(MilestonePart { to: mission_tip.to_string(), error: None, ..part.clone() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn part(branch: &str, base: &str, pr: Option<&str>, merged: bool) -> MilestonePart {
        MilestonePart {
            branch: branch.into(),
            from: "a".into(),
            to: "b".into(),
            base: base.into(),
            base_commit: "c".into(),
            onto: "main".into(),
            pr: pr.map(str::to_string),
            merged,
            error: None,
        }
    }

    fn mission(parts: &[(&str, MilestoneState, Option<MilestonePart>)]) -> MissionYaml {
        let mut m = MissionYaml::new("Add SSO", "Users sign in with SSO", "P");
        for (id, state, p) in parts {
            let mut ms = Milestone::new(*id, format!("{id} title"));
            ms.state = *state;
            if let Some(p) = p {
                ms.parts.insert("/repo".into(), p.clone());
            }
            m.milestones.push(ms);
        }
        m
    }

    #[test]
    fn new_missions_are_stacked_and_old_files_are_not() {
        assert!(MissionYaml::new("t", "g", "p").stacked);
        let old = "title: t\ngoal: g\nproject: p\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n";
        let parsed: MissionYaml = serde_yaml::from_str(old).unwrap();
        assert!(!parsed.stacked, "a mission from before stacking keeps its single pull request");
        assert!(series(&parsed).is_empty());
    }

    #[test]
    fn the_series_skips_dropped_and_empty_milestones() {
        let m = mission(&[
            ("M1", MilestoneState::Passed, Some(part("tendril/m1", "main", Some("https://github.com/o/r/pull/1"), false))),
            ("M2", MilestoneState::Skipped, None),
            ("M3", MilestoneState::Passed, None),
            ("M4", MilestoneState::Executing, None),
            ("M5", MilestoneState::Pending, None),
        ]);
        let ids: Vec<&str> = series(&m).iter().map(|ms| ms.id.as_str()).collect();
        assert_eq!(ids, vec!["M1", "M4", "M5"]);
        assert_eq!(position(&m, "M4"), Some((2, 3)));
        assert_eq!(position(&m, "M2"), None);
        assert_eq!(pr_title(&m, "00012", m.milestone("M4").unwrap()), "[00012] Add SSO (part 2 of 3): M4 title");
    }

    #[test]
    fn the_series_section_links_every_part() {
        let m = mission(&[
            ("M1", MilestoneState::Passed, Some(part("tendril/m1", "main", Some("https://github.com/o/r/pull/1"), false))),
            ("M2", MilestoneState::Passed, Some(part("tendril/m2", "tendril/m1", None, false))),
            ("M3", MilestoneState::Pending, None),
        ]);
        let section = series_section(&m, "M2", "/repo").unwrap();
        assert!(section.starts_with("### Part 2 of 3: Add SSO"), "{section}");
        assert!(section.contains("1. [M1] M1 title - https://github.com/o/r/pull/1"));
        assert!(section.contains("2. **[M2] M2 title** (this PR)"));
        assert!(section.contains("3. [M3] M3 title - not opened yet"));
        let body = pr_body(&m, m.milestone("M2").unwrap(), "/repo");
        assert!(body.starts_with("### Part 2 of 3") && body.contains("## M2 — M2 title"));
    }

    #[test]
    fn parts_open_in_order_on_the_open_part_below() {
        let m = mission(&[
            ("M1", MilestoneState::Passed, Some(part("tendril/m1", "main", Some("u1"), true))),
            ("M2", MilestoneState::Passed, Some(part("tendril/m2", "main", Some("u2"), false))),
            ("M3", MilestoneState::Passed, Some(part("tendril/m3", "", None, false))),
            ("M4", MilestoneState::Passed, Some(part("tendril/m4", "", None, false))),
        ]);
        assert_eq!(next_to_open(&m, "/repo").unwrap().id, "M3");
        assert_eq!(part_below(&m, "M3", "/repo").unwrap().branch, "tendril/m2");
        assert!(part_below(&m, "M2", "/repo").is_none(), "M1 merged: M2 builds on the base branch");
        assert!(next_to_open(&m, "/other").is_none());
        assert!(has_open_parts(&m) && !all_merged(&m));
    }

    #[test]
    fn a_merged_part_moves_the_next_onto_the_base_and_replays_the_rest() {
        let m = mission(&[
            ("M1", MilestoneState::Passed, Some(part("tendril/m1", "main", Some("u1"), true))),
            ("M2", MilestoneState::Passed, Some(part("tendril/m2", "tendril/m1", Some("u2"), false))),
            ("M3", MilestoneState::Passed, Some(part("tendril/m3", "tendril/m2", Some("u3"), false))),
            ("M4", MilestoneState::Passed, Some(part("tendril/m4", "", None, false))),
        ]);
        assert_eq!(
            restack_plan(&m, "/repo"),
            vec![
                Restack { milestone: "M2".into(), new_base: "main".into(), onto_base_branch: true },
                Restack { milestone: "M3".into(), new_base: "tendril/m2".into(), onto_base_branch: false },
            ]
        );
        // Nothing merged: nothing moves.
        let quiet = mission(&[
            ("M1", MilestoneState::Passed, Some(part("tendril/m1", "main", Some("u1"), false))),
            ("M2", MilestoneState::Passed, Some(part("tendril/m2", "tendril/m1", Some("u2"), false))),
        ]);
        assert!(restack_plan(&quiet, "/repo").is_empty());
    }

    #[test]
    fn a_middle_part_merging_moves_only_what_is_above_it() {
        let m = mission(&[
            ("M1", MilestoneState::Passed, Some(part("tendril/m1", "main", Some("u1"), false))),
            ("M2", MilestoneState::Passed, Some(part("tendril/m2", "tendril/m1", Some("u2"), true))),
            ("M3", MilestoneState::Passed, Some(part("tendril/m3", "tendril/m2", Some("u3"), false))),
        ]);
        assert_eq!(
            restack_plan(&m, "/repo"),
            vec![Restack { milestone: "M3".into(), new_base: "tendril/m1".into(), onto_base_branch: false }]
        );
    }

    #[test]
    fn a_part_that_failed_to_move_is_tried_again() {
        let mut stuck = part("tendril/m3", "tendril/m2", Some("u3"), false);
        stuck.error = Some("conflicts".into());
        let m = mission(&[
            ("M2", MilestoneState::Passed, Some(part("tendril/m2", "main", Some("u2"), false))),
            ("M3", MilestoneState::Passed, Some(stuck)),
        ]);
        assert_eq!(
            restack_plan(&m, "/repo"),
            vec![Restack { milestone: "M3".into(), new_base: "tendril/m2".into(), onto_base_branch: false }]
        );
    }

    fn g(dir: &Path, args: &[&str]) -> String {
        let (code, out, err) = run_git(args, dir).unwrap();
        assert_eq!(code, 0, "git {args:?}: {err}");
        out.trim().to_string()
    }

    fn commit_file(dir: &Path, file: &str, text: &str) -> String {
        std::fs::write(dir.join(file), text).unwrap();
        g(dir, &["add", "."]);
        g(dir, &["-c", "commit.gpgsign=false", "commit", "-q", "-m", file]);
        g(dir, &["rev-parse", "HEAD"])
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &["config", "user.email", "t@example.com"]);
        g(&repo, &["config", "user.name", "T"]);
        g(&repo, &["config", "commit.gpgsign", "false"]);
        commit_file(&repo, "root.txt", "root");
        (dir, repo)
    }

    #[test]
    fn replay_points_at_the_tip_when_it_already_sits_on_the_base() {
        let (_d, repo) = repo();
        let base = g(&repo, &["rev-parse", "HEAD"]);
        g(&repo, &["checkout", "-q", "-b", "mission"]);
        let one = commit_file(&repo, "one.txt", "1");
        let scratch = repo.parent().unwrap().join("scratch");
        assert_eq!(replay(&repo, "part-1", &base, &one, "main", &scratch).unwrap(), one);
        assert_eq!(g(&repo, &["rev-parse", "part-1"]), one);
    }

    #[test]
    fn replay_rebuilds_a_part_on_a_rewritten_part_below() {
        let (_d, repo) = repo();
        let root = g(&repo, &["rev-parse", "HEAD"]);
        g(&repo, &["checkout", "-q", "-b", "mission"]);
        let one = commit_file(&repo, "one.txt", "1");
        let two = commit_file(&repo, "two.txt", "2");
        g(&repo, &["checkout", "-q", "main"]);
        let scratch = repo.parent().unwrap().join("scratch");
        replay(&repo, "part-1", &root, &one, "main", &scratch).unwrap();
        // Part 1 is squash-merged into main: part 2 replays its own commit onto main alone.
        std::fs::write(repo.join("one.txt"), "1").unwrap();
        g(&repo, &["add", "."]);
        g(&repo, &["commit", "-q", "-m", "Part 1 (#1)"]);
        let main = g(&repo, &["rev-parse", "main"]);
        let tip = replay(&repo, "part-2", &one, &two, "main", &scratch).unwrap();
        assert_ne!(tip, two);
        assert_eq!(g(&repo, &["rev-list", "--count", &format!("{main}..{tip}")]), "1");
        assert_eq!(g(&repo, &["show", "--format=", "--name-only", &tip]), "two.txt");
        assert!(!scratch.exists(), "the scratch checkout is removed");
    }

    #[test]
    fn a_conflicting_replay_leaves_the_branch_alone() {
        let (_d, repo) = repo();
        let root = g(&repo, &["rev-parse", "HEAD"]);
        g(&repo, &["checkout", "-q", "-b", "mission"]);
        let one = commit_file(&repo, "same.txt", "mission");
        g(&repo, &["branch", "part-1", &one]);
        g(&repo, &["checkout", "-q", "main"]);
        commit_file(&repo, "same.txt", "main");
        let scratch = repo.parent().unwrap().join("scratch");
        assert!(replay(&repo, "part-1", &root, &one, "main", &scratch).unwrap_err().contains("conflicts"));
        assert_eq!(g(&repo, &["rev-parse", "part-1"]), one);
        assert!(!scratch.exists());
    }

    #[derive(Default)]
    struct FakeGh {
        created: std::sync::Mutex<Vec<(String, String, String)>>,
        comments: std::sync::Mutex<Vec<(u64, String)>>,
    }

    impl GitHub for FakeGh {
        fn repo_slug(&self, remote_url: &str) -> Option<String> {
            (!remote_url.is_empty()).then(|| "o/r".to_string())
        }
        fn default_branch(&self, _: &str) -> crate::error::Result<String> {
            Ok("main".into())
        }
        fn branch_visible(&self, _: &str, _: &str) -> bool {
            true
        }
        fn open_pr_for_branch(&self, _: &str, _: &str) -> crate::error::Result<Option<crate::pull_request::PrRef>> {
            Ok(None)
        }
        fn create_pr(&self, pr: &NewPr<'_>) -> crate::error::Result<crate::pull_request::PrRef> {
            let mut created = self.created.lock().unwrap();
            created.push((pr.base.to_string(), pr.head.to_string(), pr.body.to_string()));
            let number = created.len() as u64;
            Ok(crate::pull_request::PrRef { number, url: format!("https://github.com/o/r/pull/{number}") })
        }
        fn comment(&self, _: &str, number: u64, body: &str) -> crate::error::Result<()> {
            self.comments.lock().unwrap().push((number, body.to_string()));
            Ok(())
        }
        fn mergeable(&self, _: &str, _: u64) -> crate::error::Result<String> {
            Ok("MERGEABLE".into())
        }
        fn merge(&self, _: &str, _: u64, _: bool) -> crate::error::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parts_open_stacked_and_move_onto_main_after_a_squash_merge() {
        let (dir, repo) = repo();
        let remote = dir.path().join("remote.git");
        g(dir.path(), &["init", "-q", "--bare", "-b", "main", &remote.to_string_lossy()]);
        g(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
        g(&repo, &["push", "-q", "origin", "main"]);
        let root = g(&repo, &["rev-parse", "HEAD"]);
        g(&repo, &["checkout", "-q", "-b", "mission"]);
        let one = commit_file(&repo, "one.txt", "1");
        let two = commit_file(&repo, "two.txt", "2");
        g(&repo, &["checkout", "-q", "main"]);
        let scratch = dir.path().join("scratch");
        let gh = FakeGh::default();

        let p1 = MilestonePart { from: root.clone(), to: one.clone(), ..part("tendril/m1", "", None, false) };
        let Opened::Opened(p1) = open_part(&gh, &repo, &p1, None, "main", "t1", "b1", &scratch, false) else { panic!("part 1 did not open") };
        assert_eq!((p1.base.as_str(), p1.base_commit.as_str()), ("main", root.as_str()));
        let p2 = MilestonePart { from: one.clone(), to: two.clone(), ..part("tendril/m2", "", None, false) };
        let Opened::Opened(p2) = open_part(&gh, &repo, &p2, Some(&p1), "main", "t2", "b2", &scratch, false) else { panic!("part 2 did not open") };
        assert_eq!((p2.base.as_str(), p2.base_commit.as_str()), ("tendril/m1", one.as_str()));
        {
            let created = gh.created.lock().unwrap();
            assert_eq!(created[0].0, "main");
            assert_eq!((created[1].0.as_str(), created[1].1.as_str()), ("tendril/m1", "tendril/m2"));
            assert!(created[1].2.starts_with("b2") && created[1].2.contains("two.txt"), "{}", created[1].2);
        }
        assert_eq!(gh.comments.lock().unwrap()[0].0, 1, "part 1's PR links to the next part");

        // Part 1 is squash-merged on GitHub: part 2 moves onto main, carrying only its own commit.
        let other = dir.path().join("other");
        g(dir.path(), &["clone", "-q", &remote.to_string_lossy(), &other.to_string_lossy()]);
        g(&other, &["config", "user.email", "t@example.com"]);
        g(&other, &["config", "user.name", "T"]);
        std::fs::write(other.join("one.txt"), "1").unwrap();
        g(&other, &["add", "."]);
        g(&other, &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "Part 1 (#1)"]);
        g(&other, &["push", "-q", "origin", "main"]);

        let m = mission(&[
            ("M1", MilestoneState::Passed, Some(MilestonePart { merged: true, ..p1 })),
            ("M2", MilestoneState::Passed, Some(p2.clone())),
        ]);
        let steps = restack_plan(&m, "/repo");
        assert_eq!(steps, vec![Restack { milestone: "M2".into(), new_base: "main".into(), onto_base_branch: true }]);
        let retargeted = std::sync::Mutex::new(Vec::new());
        let moved = move_part(&repo, &p2, &steps[0], &scratch, false, &|url, base| {
            retargeted.lock().unwrap().push((url.to_string(), base.to_string()));
            Ok(())
        })
        .unwrap();
        let main = g(&repo, &["rev-parse", "origin/main"]);
        assert_eq!((moved.base.as_str(), moved.base_commit.as_str()), ("main", main.as_str()));
        assert_eq!(g(&repo, &["rev-list", "--count", &format!("{main}..tendril/m2")]), "1");
        assert_eq!(g(&repo, &["rev-parse", "tendril/m2"]), g(&repo, &["rev-parse", "origin/tendril/m2"]), "pushed");
        assert_eq!(retargeted.lock().unwrap()[0], ("https://github.com/o/r/pull/2".to_string(), "main".to_string()));
    }

    #[test]
    fn base_branch_prefers_the_configured_one() {
        let (_d, repo) = repo();
        assert_eq!(base_branch(&repo, Some(" develop ")), "develop");
        assert_eq!(base_branch(&repo, None), "main");
    }
}
