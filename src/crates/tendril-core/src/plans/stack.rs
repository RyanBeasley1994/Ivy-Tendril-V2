//! Stacked plans: one feature delivered as a chain of small pull requests.
//!
//! A plan `stackedOn` another is the next part of the same feature. It does not wait for the part
//! before it to merge: as soon as that part has committed, this one is cut from its branch, and its PR
//! is opened against that branch, so each PR shows only its own part and the whole chain can be
//! reviewed at once, in order.
//!
//! Where a part was cut from is recorded per repo under `stackBase` in its `plan.yaml`: the parent's
//! branch, the commit it was at, and the repo's real base branch. That record is what the firmware,
//! the PR step and the restack read, so none of them needs GitHub to know what a part is built on.
//!
//! When a part's PR merges, [`restack_children`] moves the next part onto the base branch: its
//! commits are replayed with `git rebase --onto <base> <recorded commit>` (which is right whether the
//! parent was squashed, rebased or merged), pushed, and its PR retargeted. A conflict is left for a
//! person, with the rebase aborted so nothing is half-done. Parts further down the chain are replayed
//! onto the rewritten branch the same way.

use crate::error::Result;
use crate::git::service::run_git;
use crate::git::worktree::{derive_branch_name, enumerate_worktree_directories, resolve_repo_root};
use crate::models::{PlanStatus, PlanYaml};
use crate::plans::dependencies::PrStateResolver;
use crate::plans::reader::read_plan_yaml;
use crate::plans::writer::write_plan_yaml;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STACKED_ON_KEY: &str = "stackedOn";
pub const STACK_BASE_KEY: &str = "stackBase";

/// Where one repo of a stacked part was cut from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackBase {
    /// The parent part's branch this part is built on, and its PR targets.
    pub branch: String,
    /// The commit of that branch this part was cut from: what a restack replays from.
    pub commit: String,
    /// The repo's own base branch, which this part moves onto once the parent merges.
    pub onto: String,
}

/// The plan folder name this plan is stacked on, if any.
pub fn stacked_on(plan: &PlanYaml) -> Option<String> {
    plan.extra
        .get(STACKED_ON_KEY)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Marks `plan_folder` as the part after `parent` (a sibling folder name).
pub fn set_stacked_on(plan_folder: &Path, parent: &str) -> Result<()> {
    let (mut plan, _) = read_plan_yaml(plan_folder)?;
    plan.extra.insert(
        STACKED_ON_KEY.to_string(),
        serde_yaml::Value::String(parent.trim().to_string()),
    );
    write_plan_yaml(plan_folder, &plan)
}

/// The recorded cut points, keyed by repo root.
pub fn stack_bases(plan: &PlanYaml) -> BTreeMap<String, StackBase> {
    plan.extra
        .get(STACK_BASE_KEY)
        .and_then(|v| serde_yaml::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// The cut point recorded for `repo`, compared by canonical path.
pub fn stack_base_for(plan: &PlanYaml, repo: &Path) -> Option<StackBase> {
    let want = canonical(repo);
    stack_bases(plan)
        .into_iter()
        .find(|(path, _)| canonical(Path::new(path)) == want)
        .map(|(_, base)| base)
}

fn write_stack_bases(plan_folder: &Path, update: impl FnOnce(&mut BTreeMap<String, StackBase>)) -> Result<()> {
    let (mut plan, _) = read_plan_yaml(plan_folder)?;
    let mut bases = stack_bases(&plan);
    update(&mut bases);
    if bases.is_empty() {
        plan.extra.remove(STACK_BASE_KEY);
    } else {
        let value = serde_yaml::to_value(&bases)
            .map_err(|e| crate::error::TendrilError::Plan(format!("Failed to serialize stackBase: {e}")))?;
        plan.extra.insert(STACK_BASE_KEY.to_string(), value);
    }
    write_plan_yaml(plan_folder, &plan)
}

/// Records that `plan_folder`'s worktree for `repo` was cut from `base`.
pub fn record_stack_base(plan_folder: &Path, repo: &Path, base: StackBase) -> Result<()> {
    let key = repo.to_string_lossy().to_string();
    write_stack_bases(plan_folder, |bases| {
        bases.insert(key, base);
    })
}

/// Where the part before this one has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parent {
    /// Not stacked.
    None,
    /// Merged (or completed without a PR): build on the repo's base branch as usual.
    Merged,
    /// Committed but not merged: build on its branch, in the repos it worked in.
    Open { folder: PathBuf, branch: String, repos: Vec<String> },
    /// Not ready to build on yet, and why.
    Waiting(String),
}

pub fn parent_status(plans_dir: &Path, plan: &PlanYaml, resolve_pr_state: PrStateResolver) -> Parent {
    let Some(name) = stacked_on(plan) else {
        return Parent::None;
    };
    let folder = plans_dir.join(&name);
    let Ok((parent, _)) = read_plan_yaml(&folder) else {
        return Parent::Waiting(format!("The part this builds on ('{name}') does not exist"));
    };
    let state = PlanStatus::from_str_loose(&parent.state);
    let prs: Vec<&String> = parent.prs.iter().filter(|u| u.contains("/pull/")).collect();
    let all_merged = !prs.is_empty()
        && prs
            .iter()
            .all(|u| resolve_pr_state(u).is_ok_and(|s| s.eq_ignore_ascii_case("MERGED")));
    if all_merged || (state == Some(PlanStatus::Completed) && prs.is_empty()) {
        return Parent::Merged;
    }
    let committed = !parent.commits.is_empty() || !prs.is_empty();
    match state {
        Some(PlanStatus::Review | PlanStatus::Completed) if committed => {
            let repos = parent
                .worktrees
                .as_ref()
                .map(|w| w.iter().map(|e| e.repo.clone()).collect::<Vec<_>>())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| parent.repos.clone());
            Parent::Open {
                branch: derive_branch_name(&folder),
                folder,
                repos,
            }
        }
        _ => Parent::Waiting(format!(
            "Waiting for the part this builds on ('{name}') to finish; it is {}",
            parent.state
        )),
    }
}

impl Parent {
    /// The parent's branch to cut `repo` from, when the parent is open and worked in that repo.
    pub fn branch_for(&self, repo: &Path) -> Option<&str> {
        match self {
            Parent::Open { branch, repos, .. } => {
                let want = canonical(repo);
                repos
                    .iter()
                    .any(|r| canonical(Path::new(r)) == want)
                    .then_some(branch.as_str())
            }
            _ => None,
        }
    }
}

/// The branch to cut `plan_folder`'s worktree for `repo` from, when it is a stacked part: the
/// recorded parent branch for a part already cut (a retry must not move it), else the open parent's.
/// `None` means cut from the repo's base branch as usual.
pub fn cut_from(plan_folder: &Path, repo: &Path, resolve_pr_state: PrStateResolver) -> Option<String> {
    let (plan, _) = read_plan_yaml(plan_folder).ok()?;
    if let Some(recorded) = stack_base_for(&plan, repo) {
        return Some(recorded.branch);
    }
    let plans_dir = plan_folder.parent()?;
    parent_status(plans_dir, &plan, resolve_pr_state)
        .branch_for(repo)
        .map(str::to_string)
}

/// Records where a freshly cut worktree of a stacked part came from: `branch` at the worktree's
/// current commit, moving onto `onto` (the repo's base branch) once the parent merges.
pub fn record_cut(plan_folder: &Path, repo: &Path, worktree: &Path, branch: &str, onto: &str) -> Result<()> {
    let (_, commit, _) = run_git(&["rev-parse", "HEAD"], worktree)?;
    record_stack_base(
        plan_folder,
        repo,
        StackBase { branch: branch.to_string(), commit: commit.trim().to_string(), onto: onto.to_string() },
    )
}

/// The plans stacked directly on `folder_name`, lowest id first.
pub fn children(plans_dir: &Path, folder_name: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(plans_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("plan.yaml").exists())
                .filter(|p| {
                    read_plan_yaml(p)
                        .ok()
                        .and_then(|(plan, _)| stacked_on(&plan))
                        .is_some_and(|parent| parent == folder_name)
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// The whole chain `plan_folder` belongs to, first part to last. A plan in no chain is alone in it.
pub fn series(plans_dir: &Path, plan_folder: &Path) -> Vec<PathBuf> {
    let mut chain = vec![plan_folder.to_path_buf()];
    // Up to the first part, guarding against a loop someone wrote by hand.
    while let Some(parent) = read_plan_yaml(&chain[0]).ok().and_then(|(p, _)| stacked_on(&p)) {
        let folder = plans_dir.join(parent);
        if !folder.join("plan.yaml").exists() || chain.contains(&folder) {
            break;
        }
        chain.insert(0, folder);
    }
    // Down through the first child of each part.
    loop {
        let last = chain.last().expect("chain is never empty");
        let Some(next) = children(plans_dir, &folder_name(last)).into_iter().next() else {
            break;
        };
        if chain.contains(&next) {
            break;
        }
        chain.push(next);
    }
    chain
}

/// The "Part k of n" section of a stacked part's PR body, or `None` for a plan in no chain.
pub fn series_section(plans_dir: &Path, plan_folder: &Path) -> Option<String> {
    let chain = series(plans_dir, plan_folder);
    if chain.len() < 2 {
        return None;
    }
    let me = canonical(plan_folder);
    let position = chain.iter().position(|f| canonical(f) == me)? + 1;
    let feature = read_plan_yaml(&chain[0]).map(|(p, _)| p.title).unwrap_or_default();
    let mut out = format!(
        "### Part {position} of {}: {feature}\n\nOne part of a stacked series. Each PR shows only its own part; review and merge them in order.\n\n",
        chain.len()
    );
    for (i, folder) in chain.iter().enumerate() {
        let Ok((plan, _)) = read_plan_yaml(folder) else { continue };
        let id: String = folder_name(folder).chars().take(5).collect();
        let pr = plan.prs.iter().find(|u| u.contains("/pull/"));
        let line = if i + 1 == position {
            format!("{}. **[{id}] {}** (this PR)", i + 1, plan.title)
        } else {
            match pr {
                Some(url) => format!("{}. [{id}] {} - {url}", i + 1, plan.title),
                None => format!("{}. [{id}] {} - not opened yet", i + 1, plan.title),
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

/// One repo of one part, moved after the part before it merged.
#[derive(Debug, Clone)]
pub struct RestackOutcome {
    pub plan: String,
    pub repo: String,
    pub ok: bool,
    pub message: String,
}

/// Moves the parts stacked on `merged_folder` onto their repos' base branches, now it has merged.
pub fn restack_children(plans_dir: &Path, merged_folder: &Path) -> Vec<RestackOutcome> {
    let mut outcomes = Vec::new();
    for child in children(plans_dir, &folder_name(merged_folder)) {
        restack(plans_dir, &child, None, &mut outcomes);
    }
    outcomes
}

/// Replays `plan_folder`'s commits onto `new_parent` (the parent's rewritten branch), or onto each
/// repo's base branch when `None`, then does the same for the parts stacked on it.
fn restack(plans_dir: &Path, plan_folder: &Path, new_parent: Option<&str>, outcomes: &mut Vec<RestackOutcome>) {
    let Ok((plan, _)) = read_plan_yaml(plan_folder) else { return };
    let bases = stack_bases(&plan);
    let name = folder_name(plan_folder);
    let mut moved_any = false;

    for worktree in enumerate_worktree_directories(&plan_folder.join("Worktrees")) {
        let Some(repo_root) = resolve_repo_root(&worktree) else { continue };
        let want = canonical(&repo_root);
        let Some((key, base)) = bases
            .iter()
            .find(|(path, _)| canonical(Path::new(path)) == want)
            .map(|(k, b)| (k.clone(), b.clone()))
        else {
            continue;
        };
        let repo = repo_root.to_string_lossy().to_string();
        let mut report = |ok: bool, message: String| {
            outcomes.push(RestackOutcome { plan: name.clone(), repo: repo.clone(), ok, message });
        };

        let git = |args: &[&str]| -> std::result::Result<String, String> {
            match run_git(args, &worktree) {
                Ok((0, out, _)) => Ok(out.trim().to_string()),
                Ok((_, _, err)) => Err(err.trim().to_string()),
                Err(e) => Err(e.to_string()),
            }
        };
        if !git(&["status", "--porcelain"]).unwrap_or_default().is_empty() {
            report(false, "has uncommitted changes, so it was not moved".into());
            continue;
        }
        let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
        let _ = git(&["fetch", "origin"]);
        let onto = match new_parent {
            Some(parent) => parent.to_string(),
            None => format!("origin/{}", base.onto),
        };
        if let Err(e) = git(&["rebase", "--onto", &onto, &base.commit]) {
            let _ = git(&["rebase", "--abort"]);
            report(false, format!("conflicts moving onto {onto}; rebase it by hand ({e})"));
            continue;
        }
        let pushed = plan.prs.iter().any(|u| u.contains("/pull/"));
        if pushed {
            if let Err(e) = git(&["push", "--force-with-lease", "origin", &branch]) {
                report(false, format!("moved onto {onto} but the push failed: {e}"));
                continue;
            }
        }
        moved_any = true;

        if new_parent.is_none() {
            // On the base branch now: no longer built on anything, and the PR says so.
            let _ = write_stack_bases(plan_folder, |b| {
                b.remove(&key);
            });
            for url in plan.prs.iter().filter(|u| u.contains("/pull/")) {
                let _ = std::process::Command::new("gh")
                    .args(["pr", "edit", url, "--base", &base.onto])
                    .output();
            }
            report(true, format!("moved onto {}", base.onto));
        } else {
            let tip = run_git(&["rev-parse", new_parent.unwrap_or_default()], &worktree)
                .ok()
                .map(|(_, out, _)| out.trim().to_string())
                .unwrap_or_default();
            let _ = write_stack_bases(plan_folder, |b| {
                if let Some(entry) = b.get_mut(&key) {
                    entry.commit = tip;
                }
            });
            report(true, format!("replayed onto the rewritten {onto}"));
        }
    }

    if !moved_any {
        return;
    }
    let branch = derive_branch_name(plan_folder);
    for child in children(plans_dir, &name) {
        restack(plans_dir, &child, Some(&branch), outcomes);
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_in(dir: &Path, name: &str, state: &str, parent: Option<&str>, prs: &[&str]) -> PathBuf {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        let mut plan = PlanYaml { title: name.to_string(), state: state.to_string(), ..Default::default() };
        plan.prs = prs.iter().map(|s| s.to_string()).collect();
        plan.commits = vec!["abc".into()];
        plan.repos = vec!["/repo".into()];
        if let Some(p) = parent {
            plan.extra.insert(STACKED_ON_KEY.into(), serde_yaml::Value::String(p.into()));
        }
        write_plan_yaml(&folder, &plan).unwrap();
        folder
    }

    #[test]
    fn a_part_builds_on_an_open_parent_and_on_the_base_once_it_merges() {
        let dir = tempfile::tempdir().unwrap();
        plan_in(dir.path(), "00001-One", "Review", None, &["https://github.com/o/r/pull/1"]);
        let two = plan_in(dir.path(), "00002-Two", "Draft", Some("00001-One"), &[]);
        let (plan, _) = read_plan_yaml(&two).unwrap();

        let open = parent_status(dir.path(), &plan, &|_| Ok("OPEN".into()));
        assert_eq!(open.branch_for(Path::new("/repo")), Some("tendril/00001-One"));
        assert_eq!(open.branch_for(Path::new("/other")), None);
        assert_eq!(parent_status(dir.path(), &plan, &|_| Ok("MERGED".into())), Parent::Merged);
    }

    #[test]
    fn a_part_waits_while_its_parent_is_still_running() {
        let dir = tempfile::tempdir().unwrap();
        plan_in(dir.path(), "00001-One", "Executing", None, &[]);
        let two = plan_in(dir.path(), "00002-Two", "Draft", Some("00001-One"), &[]);
        let (plan, _) = read_plan_yaml(&two).unwrap();
        assert!(matches!(parent_status(dir.path(), &plan, &|_| Ok("OPEN".into())), Parent::Waiting(_)));
    }

    #[test]
    fn the_series_is_listed_in_order_from_any_part() {
        let dir = tempfile::tempdir().unwrap();
        let one = plan_in(dir.path(), "00001-One", "Review", None, &["https://github.com/o/r/pull/1"]);
        let two = plan_in(dir.path(), "00002-Two", "Review", Some("00001-One"), &[]);
        let three = plan_in(dir.path(), "00003-Three", "Draft", Some("00002-Two"), &[]);
        assert_eq!(series(dir.path(), &two), vec![one.clone(), two.clone(), three]);

        let section = series_section(dir.path(), &two).unwrap();
        assert!(section.starts_with("### Part 2 of 3: 00001-One"));
        assert!(section.contains("1. [00001] 00001-One - https://github.com/o/r/pull/1"));
        assert!(section.contains("**[00002] 00002-Two** (this PR)"));
        assert!(section.contains("3. [00003] 00003-Three - not opened yet"));
        assert!(series_section(dir.path(), &dir.path().join("missing")).is_none());
    }
}
