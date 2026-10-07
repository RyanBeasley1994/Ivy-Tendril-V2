//! A repository's working-tree status, from `git status --porcelain=v2 --branch -z`.

use super::{git_dir, git_read_ok, RepoState};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub path: String,
    /// The old path, for a rename or copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    /// The index (staged) side: `M`odified, `A`dded, `D`eleted, `R`enamed, `C`opied, `T`ypechange, or `.`.
    pub index: String,
    /// The working-tree (unstaged) side, same letters.
    pub worktree: String,
    pub conflicted: bool,
    pub untracked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    /// `None` while the head is detached or the repository has no commit yet.
    pub branch: Option<String>,
    pub detached: bool,
    /// The full hash HEAD points at; `None` in a repository with no commits.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<StatusEntry>,
    pub state: RepoState,
}

impl RepoStatus {
    pub fn staged(&self) -> u32 {
        self.entries.iter().filter(|e| !e.conflicted && !e.untracked && e.index != ".").count() as u32
    }

    pub fn unstaged(&self) -> u32 {
        self.entries.iter().filter(|e| !e.conflicted && !e.untracked && e.worktree != ".").count() as u32
    }

    pub fn untracked(&self) -> u32 {
        self.entries.iter().filter(|e| e.untracked).count() as u32
    }

    pub fn conflicted(&self) -> u32 {
        self.entries.iter().filter(|e| e.conflicted).count() as u32
    }

    pub fn is_dirty(&self) -> bool {
        !self.entries.is_empty()
    }
}

/// Parses the NUL-separated porcelain v2 output. A rename entry (`2`) is followed by one extra field: the
/// path it came from.
pub fn parse_status_v2(stdout: &str) -> RepoStatus {
    let mut status = RepoStatus::default();
    let mut fields = stdout.split('\0').filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        if let Some(header) = field.strip_prefix("# ") {
            let mut parts = header.splitn(2, ' ');
            match (parts.next(), parts.next()) {
                (Some("branch.oid"), Some(oid)) => {
                    status.head = (oid != "(initial)").then(|| oid.to_string());
                }
                (Some("branch.head"), Some(name)) => {
                    if name == "(detached)" {
                        status.detached = true;
                    } else {
                        status.branch = Some(name.to_string());
                    }
                }
                (Some("branch.upstream"), Some(up)) => status.upstream = Some(up.to_string()),
                (Some("branch.ab"), Some(ab)) => {
                    for token in ab.split_whitespace() {
                        if let Some(n) = token.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = token.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
        } else if let Some(path) = field.strip_prefix("? ") {
            status.entries.push(StatusEntry {
                path: path.to_string(),
                orig_path: None,
                index: ".".into(),
                worktree: "?".into(),
                conflicted: false,
                untracked: true,
            });
        } else if field.starts_with("1 ") {
            // 1 XY sub mH mI mW hH hI path
            let parts: Vec<&str> = field.splitn(9, ' ').collect();
            if parts.len() == 9 {
                status.entries.push(entry(parts[1], parts[8], None, false));
            }
        } else if field.starts_with("2 ") {
            // 2 XY sub mH mI mW hH hI Xscore path, then the original path as the next field
            let parts: Vec<&str> = field.splitn(10, ' ').collect();
            let orig = fields.next().map(str::to_string);
            if parts.len() == 10 {
                status.entries.push(entry(parts[1], parts[9], orig, false));
            }
        } else if field.starts_with("u ") {
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            let parts: Vec<&str> = field.splitn(11, ' ').collect();
            if parts.len() == 11 {
                status.entries.push(entry(parts[1], parts[10], None, true));
            }
        }
    }
    status
}

fn entry(xy: &str, path: &str, orig: Option<String>, conflicted: bool) -> StatusEntry {
    let mut chars = xy.chars();
    StatusEntry {
        path: path.to_string(),
        orig_path: orig,
        index: chars.next().unwrap_or('.').to_string(),
        worktree: chars.next().unwrap_or('.').to_string(),
        conflicted,
        untracked: false,
    }
}

/// What the repository is in the middle of, read from the marker files git leaves in its git directory.
pub fn repo_state(git_dir: &Path) -> RepoState {
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        RepoState::Rebasing
    } else if git_dir.join("MERGE_HEAD").exists() {
        RepoState::Merging
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        RepoState::CherryPicking
    } else if git_dir.join("REVERT_HEAD").exists() {
        RepoState::Reverting
    } else if git_dir.join("BISECT_LOG").exists() {
        RepoState::Bisecting
    } else {
        RepoState::Clean
    }
}

pub fn read_status(repo: &Path) -> Result<RepoStatus> {
    let out = git_read_ok(repo, &["status", "--porcelain=v2", "--branch", "--untracked-files=all", "-z"])?;
    let mut status = parse_status_v2(&out);
    if let Ok(dir) = git_dir(repo) {
        status.state = repo_state(&dir);
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::super::testkit::Scratch;
    use super::*;

    #[test]
    fn porcelain_v2_is_parsed_including_renames_conflicts_and_untracked() {
        let raw = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
                   1 .M N... 100644 100644 100644 aaa bbb src/lib.rs\0\
                   1 A. N... 000000 100644 100644 000 ccc new file.txt\0\
                   2 R. N... 100644 100644 100644 ddd eee R100 renamed.txt\0old name.txt\0\
                   u UU N... 100644 100644 100644 100644 f1 f2 f3 both.rs\0\
                   ? scratch/notes.md\0";
        let s = parse_status_v2(raw);
        assert_eq!((s.branch.as_deref(), s.upstream.as_deref(), s.ahead, s.behind), (Some("main"), Some("origin/main"), 2, 1));
        assert_eq!(s.head.as_deref(), Some("abc123"));
        assert_eq!(s.entries.len(), 5);
        let renamed = s.entries.iter().find(|e| e.path == "renamed.txt").unwrap();
        assert_eq!(renamed.orig_path.as_deref(), Some("old name.txt"));
        assert!(s.entries.iter().any(|e| e.path == "new file.txt" && e.index == "A"), "a path with a space survives");
        assert_eq!((s.staged(), s.unstaged(), s.untracked(), s.conflicted()), (2, 1, 1, 1));
    }

    #[test]
    fn a_detached_head_and_an_empty_repository_are_reported_honestly() {
        let s = parse_status_v2("# branch.oid abc\0# branch.head (detached)\0");
        assert!(s.detached && s.branch.is_none());
        let empty = parse_status_v2("# branch.oid (initial)\0# branch.head main\0");
        assert_eq!(empty.head, None);
        assert_eq!(empty.branch.as_deref(), Some("main"));
    }

    #[test]
    fn status_of_a_real_repository_sees_staged_unstaged_and_untracked_work() {
        let repo = Scratch::new();
        repo.commit("a.txt", "one\n", "first");
        repo.commit("b.txt", "one\n", "second");
        repo.write("a.txt", "changed\n");
        repo.write("b.txt", "staged\n");
        repo.git(&["add", "b.txt"]);
        repo.write("dir/new.txt", "x\n");

        let s = read_status(&repo.dir).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!((s.staged(), s.unstaged(), s.untracked()), (1, 1, 1));
        assert!(s.is_dirty());
        assert_eq!(s.state, RepoState::Clean);
        assert!(s.entries.iter().any(|e| e.path == "dir/new.txt" && e.untracked), "untracked files are listed one by one");
    }

    #[test]
    fn a_merge_in_progress_is_recognised_from_its_marker_file() {
        let dir = std::env::temp_dir().join(format!("tendril-state-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(repo_state(&dir), RepoState::Clean);
        std::fs::write(dir.join("MERGE_HEAD"), "x").unwrap();
        assert_eq!(repo_state(&dir), RepoState::Merging);
        std::fs::create_dir_all(dir.join("rebase-merge")).unwrap();
        assert_eq!(repo_state(&dir), RepoState::Rebasing, "a rebase wins over a merge marker");
        let _ = std::fs::remove_dir_all(dir);
    }
}
