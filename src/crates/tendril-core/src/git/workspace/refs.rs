//! Branches, remote branches, tags, stashes and worktrees, plus the one-line summary a repo card shows.

use super::{git_read_ok, read_status, RepoState, RepoStatus};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefInfo {
    /// `main`, `origin/main`, `v1.2`.
    pub name: String,
    pub full_ref: String,
    pub kind: RefKind,
    /// The commit it points at (a tag is peeled to its commit).
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream it tracked has been deleted on the remote.
    pub gone: bool,
    pub current: bool,
    /// The tip's date, RFC 3339.
    pub updated: String,
    pub subject: String,
    pub author: String,
    /// Already merged into the checked-out HEAD, so it is safe to delete. Only set for local branches.
    pub merged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashInfo {
    pub index: u32,
    pub message: String,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub head: String,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
    /// The repository's own checkout, as opposed to one added with `git worktree add`.
    pub main: bool,
}

/// The owner segment of a git remote URL: `github.com/<owner>/<repo>` in https, ssh and scp forms.
pub fn remote_owner(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((_, rest)) = url.split_once(':') {
        rest
    } else {
        return None;
    };
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let owner = segments.next()?;
    // `owner/repo` needs a repo after it; a lone segment is not an owner.
    segments.next()?;
    Some(owner.to_string())
}

/// The owner of the repository's `origin` remote, or `None` for a local-only repository.
pub fn repo_owner(repo: &Path) -> Option<String> {
    git_read_ok(repo, &["remote", "get-url", "origin"]).ok().and_then(|u| remote_owner(&u))
}

/// `[ahead 1, behind 2]`, `[ahead 3]`, `[gone]` or empty.
pub fn parse_track(track: &str) -> (u32, u32, bool) {
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    if inner == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

const REF_FORMAT: &str = "%(refname)%1f%(if)%(*objectname)%(then)%(*objectname)%(else)%(objectname)%(end)%1f%(upstream:short)%1f%(upstream:track)%1f%(HEAD)%1f%(creatordate:iso-strict)%1f%(contents:subject)%1f%(authorname)";

/// Parses `for-each-ref` output made with [`REF_FORMAT`]; `merged` names the local refs already in HEAD.
pub fn parse_refs(output: &str, merged: &std::collections::HashSet<String>) -> Vec<RefInfo> {
    let mut refs = Vec::new();
    for line in output.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        if f.len() < 8 {
            continue;
        }
        let full = f[0];
        let (kind, name) = if let Some(n) = full.strip_prefix("refs/heads/") {
            (RefKind::Local, n)
        } else if let Some(n) = full.strip_prefix("refs/remotes/") {
            // `origin/HEAD` is a pointer to the default branch, not a branch of its own.
            if n.ends_with("/HEAD") {
                continue;
            }
            (RefKind::Remote, n)
        } else if let Some(n) = full.strip_prefix("refs/tags/") {
            (RefKind::Tag, n)
        } else {
            continue;
        };
        let (ahead, behind, gone) = parse_track(f[3]);
        refs.push(RefInfo {
            name: name.to_string(),
            full_ref: full.to_string(),
            kind,
            hash: f[1].to_string(),
            upstream: (!f[2].is_empty()).then(|| f[2].to_string()),
            ahead,
            behind,
            gone,
            current: f[4] == "*",
            updated: f[5].to_string(),
            subject: f[6].to_string(),
            author: f[7].to_string(),
            merged: kind == RefKind::Local && merged.contains(full),
        });
    }
    refs
}

pub fn list_refs(repo: &Path) -> Result<Vec<RefInfo>> {
    let format = format!("--format={REF_FORMAT}");
    let output = git_read_ok(repo, &["for-each-ref", &format, "refs/heads", "refs/remotes", "refs/tags"])?;
    // `--merged HEAD` fails in a repository with no commits yet; no commits means nothing is merged.
    let merged: std::collections::HashSet<String> = git_read_ok(
        repo,
        &["for-each-ref", "--merged", "HEAD", "--format=%(refname)", "refs/heads"],
    )
    .map(|o| o.lines().map(str::to_string).collect())
    .unwrap_or_default();
    let mut refs = parse_refs(&output, &merged);
    refs.sort_by(|a, b| {
        (a.kind as u8, !a.current, a.name.to_lowercase()).cmp(&(b.kind as u8, !b.current, b.name.to_lowercase()))
    });
    Ok(refs)
}

pub fn list_stashes(repo: &Path) -> Result<Vec<StashInfo>> {
    let output = git_read_ok(repo, &["stash", "list", "--format=%gd%x1f%gs%x1f%at"])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\u{1f}').collect();
            if f.len() < 3 {
                return None;
            }
            let index = f[0].strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()?;
            Some(StashInfo { index, message: f[1].to_string(), at: f[2].parse().unwrap_or(0) })
        })
        .collect())
}

/// Parses `git worktree list --porcelain`: blocks separated by a blank line, the first being the main one.
pub fn parse_worktrees(output: &str) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    for (i, block) in output.split("\n\n").enumerate() {
        let mut w = WorktreeInfo {
            path: String::new(),
            branch: None,
            head: String::new(),
            detached: false,
            bare: false,
            locked: false,
            prunable: false,
            main: i == 0,
        };
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                w.path = p.to_string();
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                w.head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line == "detached" {
                w.detached = true;
            } else if line == "bare" {
                w.bare = true;
            } else if line == "locked" || line.starts_with("locked ") {
                w.locked = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                w.prunable = true;
            }
        }
        if !w.path.is_empty() {
            worktrees.push(w);
        }
    }
    worktrees
}

pub fn list_worktrees(repo: &Path) -> Result<Vec<WorktreeInfo>> {
    Ok(parse_worktrees(&git_read_ok(repo, &["worktree", "list", "--porcelain"])?))
}

/// Everything the workspace's left pane and toolbar need in one read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoOverview {
    pub status: RepoStatus,
    pub refs: Vec<RefInfo>,
    pub stashes: Vec<StashInfo>,
    pub worktrees: Vec<WorktreeInfo>,
}

pub fn read_overview(repo: &Path) -> Result<RepoOverview> {
    Ok(RepoOverview {
        status: read_status(repo)?,
        refs: list_refs(repo)?,
        stashes: list_stashes(repo).unwrap_or_default(),
        worktrees: list_worktrees(repo).unwrap_or_default(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitBrief {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub at: i64,
}

/// What a repo card shows: cheap to compute, one call per repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSummary {
    pub branch: Option<String>,
    pub detached: bool,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflicted: u32,
    pub state: RepoState,
    pub last_commit: Option<CommitBrief>,
    pub local_branches: u32,
    pub remote_branches: u32,
    pub stashes: u32,
    pub worktrees: u32,
    /// Local branches already merged into HEAD (other than HEAD's own): clutter that is safe to remove.
    pub stale_branches: u32,
}

pub fn read_summary(repo: &Path) -> Result<RepoSummary> {
    let status = read_status(repo)?;
    let refs = list_refs(repo)?;
    let last_commit = git_read_ok(repo, &["log", "-1", "--format=%H%x1f%h%x1f%s%x1f%an%x1f%at"])
        .ok()
        .and_then(|o| {
            let f: Vec<String> = o.trim_end().split('\u{1f}').map(str::to_string).collect();
            (f.len() == 5).then(|| CommitBrief {
                hash: f[0].clone(),
                short: f[1].clone(),
                subject: f[2].clone(),
                author: f[3].clone(),
                at: f[4].parse().unwrap_or(0),
            })
        });
    Ok(RepoSummary {
        branch: status.branch.clone(),
        detached: status.detached,
        head: status.head.clone(),
        upstream: status.upstream.clone(),
        ahead: status.ahead,
        behind: status.behind,
        staged: status.staged(),
        unstaged: status.unstaged(),
        untracked: status.untracked(),
        conflicted: status.conflicted(),
        state: status.state,
        last_commit,
        local_branches: refs.iter().filter(|r| r.kind == RefKind::Local).count() as u32,
        remote_branches: refs.iter().filter(|r| r.kind == RefKind::Remote).count() as u32,
        stashes: list_stashes(repo).map(|s| s.len() as u32).unwrap_or(0),
        worktrees: list_worktrees(repo).map(|w| w.len() as u32).unwrap_or(1),
        stale_branches: refs.iter().filter(|r| r.kind == RefKind::Local && r.merged && !r.current).count() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::super::testkit::Scratch;
    use super::*;

    #[test]
    fn owners_are_read_from_https_ssh_and_scp_remotes() {
        assert_eq!(remote_owner("https://github.com/acme/app.git\n").as_deref(), Some("acme"));
        assert_eq!(remote_owner("git@github.com:acme/app.git").as_deref(), Some("acme"));
        assert_eq!(remote_owner("ssh://git@github.com/acme/app").as_deref(), Some("acme"));
        assert_eq!(remote_owner("/home/dev/app"), None);
    }

    #[test]
    fn tracking_text_becomes_numbers() {
        assert_eq!(parse_track("[ahead 3, behind 2]"), (3, 2, false));
        assert_eq!(parse_track("[behind 5]"), (0, 5, false));
        assert_eq!(parse_track("[gone]"), (0, 0, true));
        assert_eq!(parse_track(""), (0, 0, false));
    }

    #[test]
    fn worktree_blocks_are_parsed_with_the_main_one_first() {
        let raw = "worktree /repo\nHEAD aaa\nbranch refs/heads/main\n\nworktree /repo/.wt/x\nHEAD bbb\nbranch refs/heads/tendril/x\nlocked\n\nworktree /tmp/gone\nHEAD ccc\ndetached\nprunable gitdir file points to non-existent location\n";
        let w = parse_worktrees(raw);
        assert_eq!(w.len(), 3);
        assert!(w[0].main && w[0].branch.as_deref() == Some("main"));
        assert!(!w[1].main && w[1].locked && w[1].branch.as_deref() == Some("tendril/x"));
        assert!(w[2].detached && w[2].prunable);
    }

    #[test]
    fn branches_tags_stashes_and_merged_state_come_from_a_real_repository() {
        let repo = Scratch::new();
        repo.commit("a.txt", "one\n", "first");
        repo.git(&["branch", "done"]);
        repo.git(&["tag", "-a", "v1", "-m", "release one"]);
        repo.git(&["checkout", "-q", "-b", "work"]);
        repo.commit("b.txt", "two\n", "work on b");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("a.txt", "dirty\n");
        repo.git(&["stash", "push", "-q", "-m", "parked"]);

        let refs = list_refs(&repo.dir).unwrap();
        let find = |n: &str| refs.iter().find(|r| r.name == n).unwrap_or_else(|| panic!("no {n}"));
        assert!(find("main").current);
        assert!(find("done").merged, "a branch at the same commit is merged");
        assert!(!find("work").merged, "a branch with its own commit is not");
        assert_eq!(find("v1").kind, RefKind::Tag);
        assert_eq!(find("v1").hash, find("main").hash, "an annotated tag is peeled to its commit");
        assert_eq!(find("work").subject, "work on b");

        let stashes = list_stashes(&repo.dir).unwrap();
        assert_eq!(stashes.len(), 1);
        assert!(stashes[0].message.contains("parked"));
        assert_eq!(list_worktrees(&repo.dir).unwrap().len(), 1);

        let s = read_summary(&repo.dir).unwrap();
        assert_eq!((s.local_branches, s.stashes, s.worktrees, s.stale_branches), (3, 1, 1, 1));
        assert_eq!(s.last_commit.unwrap().subject, "first");
        assert!(s.upstream.is_none());
    }
}
