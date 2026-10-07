//! One commit's details and files, and the diff of one file: in a commit, staged, unstaged or untracked.

use super::{git_read, git_read_ok, resolve_commit, validate_rel_path};
use crate::error::{Result, TendrilError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A diff larger than this is cut at a line and marked truncated; a page cannot show more anyway.
pub const MAX_DIFF_BYTES: usize = 400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    /// `A`dded, `M`odified, `D`eleted, `R`enamed, `C`opied, `T`ypechange.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Unix seconds.
    pub at: i64,
    pub subject: String,
    pub body: String,
    pub files: Vec<CommitFile>,
    pub additions: u32,
    pub deletions: u32,
}

/// The empty tree, whatever hash the repository uses: what a root commit is compared with.
fn empty_tree(repo: &Path) -> Result<String> {
    Ok(git_read_ok(repo, &["hash-object", "-t", "tree", "/dev/null"])?.trim().to_string())
}

/// `status\0path\0` pairs, with an extra path after `R`/`C`.
fn parse_name_status(out: &str) -> Vec<(String, String, Option<String>)> {
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let letter = status.chars().next().unwrap_or('M');
        if matches!(letter, 'R' | 'C') {
            let (old, new) = (fields.next(), fields.next());
            if let (Some(old), Some(new)) = (old, new) {
                files.push((letter.to_string(), new.to_string(), Some(old.to_string())));
            }
        } else if let Some(path) = fields.next() {
            files.push((letter.to_string(), path.to_string(), None));
        }
    }
    files
}

/// `add\tdel\tpath\0`, and for a rename `add\tdel\t\0old\0new\0`. `-` means a binary file.
fn parse_numstat(out: &str) -> HashMap<String, (u32, u32, bool)> {
    let mut stats = HashMap::new();
    let mut fields = out.split('\0');
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let mut parts = field.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let binary = add == "-" || del == "-";
        let (a, d) = (add.parse().unwrap_or(0), del.parse().unwrap_or(0));
        let key = if path.is_empty() {
            // A rename: the next two fields are the old and the new path.
            let _old = fields.next();
            fields.next().unwrap_or("").to_string()
        } else {
            path.to_string()
        };
        stats.insert(key, (a, d, binary));
    }
    stats
}

pub fn read_commit(repo: &Path, rev: &str) -> Result<CommitDetail> {
    let hash = resolve_commit(repo, rev)?;
    let meta = git_read_ok(repo, &["show", "-s", "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%b%x1e", &hash])?;
    let record = meta.split('\u{1e}').next().unwrap_or("");
    let f: Vec<&str> = record.splitn(7, '\u{1f}').collect();
    if f.len() < 7 {
        return Err(TendrilError::Git(format!("Could not read commit {rev}")));
    }
    let parents: Vec<String> = f[1].split_whitespace().map(str::to_string).collect();
    let base = match parents.first() {
        Some(p) => p.clone(),
        None => empty_tree(repo)?,
    };
    // A merge is shown against its first parent: what it brought into the branch it was made on.
    let names = git_read_ok(repo, &["diff", "-M", "-z", "--name-status", &base, &hash])?;
    let nums = parse_numstat(&git_read_ok(repo, &["diff", "-M", "-z", "--numstat", &base, &hash])?);
    let files: Vec<CommitFile> = parse_name_status(&names)
        .into_iter()
        .map(|(status, path, orig)| {
            let (additions, deletions, binary) = nums.get(&path).copied().unwrap_or((0, 0, false));
            CommitFile { path, orig_path: orig, status, additions, deletions, binary }
        })
        .collect();
    Ok(CommitDetail {
        short: hash.chars().take(7).collect(),
        hash,
        parents,
        author: f[2].to_string(),
        email: f[3].to_string(),
        at: f[4].parse().unwrap_or(0),
        subject: f[5].to_string(),
        body: f[6].trim().to_string(),
        additions: files.iter().map(|x| x.additions).sum(),
        deletions: files.iter().map(|x| x.deletions).sum(),
        files,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DiffTarget {
    Commit { hash: String },
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// The unified diff text, as git prints it.
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}

fn cap(text: String) -> (String, bool) {
    if text.len() <= MAX_DIFF_BYTES {
        return (text, false);
    }
    let mut cut = MAX_DIFF_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let end = text[..cut].rfind('\n').unwrap_or(cut);
    (text[..end].to_string(), true)
}

pub fn read_file_diff(repo: &Path, target: &DiffTarget, path: &str, orig_path: Option<&str>) -> Result<FileDiff> {
    validate_rel_path(path)?;
    if let Some(o) = orig_path {
        validate_rel_path(o)?;
    }
    let paths: Vec<&str> = match orig_path {
        Some(o) if o != path => vec![o, path],
        _ => vec![path],
    };
    let out = match target {
        DiffTarget::Commit { hash } => {
            let hash = resolve_commit(repo, hash)?;
            let base = match git_read_ok(repo, &["rev-parse", "--verify", "--quiet", &format!("{hash}^1")]) {
                Ok(p) if !p.trim().is_empty() => p.trim().to_string(),
                _ => empty_tree(repo)?,
            };
            let mut args = vec!["diff", "-M", "--no-color", "--unified=3", &base, &hash, "--"];
            args.extend(paths.iter());
            git_read(repo, &args)?
        }
        DiffTarget::Staged => {
            let mut args = vec!["diff", "--cached", "-M", "--no-color", "--unified=3", "--"];
            args.extend(paths.iter());
            git_read(repo, &args)?
        }
        DiffTarget::Unstaged => {
            let mut args = vec!["diff", "--no-color", "--unified=3", "--"];
            args.extend(paths.iter());
            git_read(repo, &args)?
        }
        // An untracked file has nothing to compare with, so compare it with nothing: every line is new.
        DiffTarget::Untracked => git_read(repo, &["diff", "--no-color", "--no-index", "--unified=3", "--", "/dev/null", path])?,
    };
    // `--no-index` exits 1 when the files differ, which here is the whole point.
    let acceptable = out.ok() || (matches!(target, DiffTarget::Untracked) && out.code == 1);
    if !acceptable {
        return Err(TendrilError::Git(out.message()));
    }
    let binary = out.stdout.contains("Binary files ") && !out.stdout.contains("\n@@ ");
    let (text, truncated) = cap(out.stdout);
    Ok(FileDiff { path: path.to_string(), text, truncated, binary })
}

/// The commits that touched one file, following it across renames, newest first.
pub fn file_history(repo: &Path, path: &str, limit: usize) -> Result<Vec<super::CommitBrief>> {
    validate_rel_path(path)?;
    let count = format!("-n{}", limit.clamp(1, 500));
    let out = git_read_ok(repo, &["log", "--follow", &count, "--format=%H%x1f%h%x1f%s%x1f%an%x1f%at", "--", path])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\u{1f}').collect();
            (f.len() == 5).then(|| super::CommitBrief {
                hash: f[0].to_string(),
                short: f[1].to_string(),
                subject: f[2].to_string(),
                author: f[3].to_string(),
                at: f[4].parse().unwrap_or(0),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::testkit::Scratch;
    use super::*;

    #[test]
    fn a_files_history_follows_it_across_a_rename() {
        let repo = Scratch::new();
        repo.commit("old.txt", "one
", "create");
        repo.git(&["mv", "old.txt", "new.txt"]);
        repo.git(&["commit", "-q", "-m", "rename"]);
        repo.commit("new.txt", "one
two
", "edit");
        repo.commit("other.txt", "x\n", "unrelated");
        let h = file_history(&repo.dir, "new.txt", 10).unwrap();
        let subjects: Vec<&str> = h.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, vec!["edit", "rename", "create"]);
        assert!(file_history(&repo.dir, "../x", 10).is_err());
    }

    #[test]
    fn name_status_and_numstat_handle_renames_and_binary() {
        let ns = parse_name_status("M\0src/a.rs\0R100\0old name.rs\0new name.rs\0A\0b.png\0");
        assert_eq!(ns[0], ("M".into(), "src/a.rs".into(), None));
        assert_eq!(ns[1], ("R".into(), "new name.rs".into(), Some("old name.rs".into())));
        assert_eq!(ns.len(), 3);
        let stats = parse_numstat("3\t1\tsrc/a.rs\0-\t-\tb.png\00\t0\t\0old name.rs\0new name.rs\0");
        assert_eq!(stats["src/a.rs"], (3, 1, false));
        assert_eq!(stats["b.png"], (0, 0, true));
        assert_eq!(stats["new name.rs"], (0, 0, false));
    }

    #[test]
    fn a_commit_lists_its_files_with_counts_including_a_root_commit_and_a_rename() {
        let repo = Scratch::new();
        let first = repo.commit("a.txt", "one\ntwo\n", "first");
        let root = read_commit(&repo.dir, &first).unwrap();
        assert_eq!(root.subject, "first");
        assert!(root.parents.is_empty());
        assert_eq!(root.files.len(), 1);
        assert_eq!((root.files[0].status.as_str(), root.files[0].additions), ("A", 2));

        repo.git(&["mv", "a.txt", "renamed.txt"]);
        repo.write("renamed.txt", "one\ntwo\nthree\n");
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", "rename and grow\n\nA longer explanation."]);
        let c = read_commit(&repo.dir, "HEAD").unwrap();
        assert_eq!(c.body, "A longer explanation.");
        assert_eq!(c.files.len(), 1);
        let f = &c.files[0];
        assert_eq!((f.status.as_str(), f.path.as_str(), f.orig_path.as_deref()), ("R", "renamed.txt", Some("a.txt")));
        assert_eq!((f.additions, f.deletions), (1, 0));
        assert_eq!((c.additions, c.deletions), (1, 0));
        assert!(read_commit(&repo.dir, "--output=x").is_err());
    }

    #[test]
    fn file_diffs_work_for_commits_staged_unstaged_and_untracked_files() {
        let repo = Scratch::new();
        let first = repo.commit("a.txt", "one\n", "first");
        let second = repo.commit("a.txt", "one\ntwo\n", "second");

        let in_commit = read_file_diff(&repo.dir, &DiffTarget::Commit { hash: second.clone() }, "a.txt", None).unwrap();
        assert!(in_commit.text.contains("+two"), "{}", in_commit.text);
        let root = read_file_diff(&repo.dir, &DiffTarget::Commit { hash: first }, "a.txt", None).unwrap();
        assert!(root.text.contains("+one") && root.text.contains("new file"));

        repo.write("a.txt", "one\ntwo\nthree\n");
        let unstaged = read_file_diff(&repo.dir, &DiffTarget::Unstaged, "a.txt", None).unwrap();
        assert!(unstaged.text.contains("+three"));
        repo.git(&["add", "a.txt"]);
        let staged = read_file_diff(&repo.dir, &DiffTarget::Staged, "a.txt", None).unwrap();
        assert!(staged.text.contains("+three"));
        assert!(read_file_diff(&repo.dir, &DiffTarget::Unstaged, "a.txt", None).unwrap().text.is_empty());

        repo.write("new/file.txt", "brand\nnew\n");
        let untracked = read_file_diff(&repo.dir, &DiffTarget::Untracked, "new/file.txt", None).unwrap();
        assert!(untracked.text.contains("+brand") && untracked.text.contains("+new"));
    }

    #[test]
    fn binary_files_hostile_paths_and_huge_diffs_are_handled() {
        let repo = Scratch::new();
        repo.commit("a.txt", "x\n", "first");
        std::fs::write(repo.dir.join("img.bin"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
        let bin = read_file_diff(&repo.dir, &DiffTarget::Untracked, "img.bin", None).unwrap();
        assert!(bin.binary);

        for hostile in ["../etc/passwd", "/etc/passwd", ""] {
            assert!(read_file_diff(&repo.dir, &DiffTarget::Unstaged, hostile, None).is_err(), "{hostile:?}");
        }

        let big = "line of text\n".repeat(60_000);
        let (cut, truncated) = cap(big.clone());
        assert!(truncated && cut.len() < big.len() && cut.ends_with("text"));
        let (small, truncated) = cap("short".into());
        assert!(!truncated && small == "short");
    }
}
