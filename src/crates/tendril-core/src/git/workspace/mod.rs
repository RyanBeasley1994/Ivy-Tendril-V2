//! The Git page's view of a repository: typed, machine-readable answers and the handful of operations a
//! person does by hand.
//!
//! Everything shells out to the `git` on the machine the daemon runs on, through its stable plumbing
//! formats (`status --porcelain=v2`, `for-each-ref --format`, `log --format`, `diff --numstat`), never by
//! scraping what it prints for humans. Three rules hold throughout:
//!
//! * **No user string is ever an option.** Names and revisions are validated first (never a leading `-`,
//!   a real ref name, a revision that resolves) and passed after `--` where git allows it.
//! * **No prompt can hang a request.** Credentials are never asked for interactively, and every call has a
//!   time limit.
//! * **Paths stay inside the repository.** A path from a client is relative, has no `..`, and is only ever
//!   joined onto the repository root.

pub mod diff;
pub mod graph;
pub mod ops;
pub mod prs;
pub mod refs;
pub mod status;

pub use diff::*;
pub use graph::*;
pub use ops::*;
pub use prs::*;
pub use refs::*;
pub use status::*;

use crate::error::{Result, TendrilError};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A read is quick; anything that touches a remote or rewrites the tree gets longer.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);

/// What a repository is in the middle of, besides being clean or dirty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RepoState {
    #[default]
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
    Bisecting,
}

#[derive(Debug, Clone)]
pub struct GitOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GitOut {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// The error text a person should see: git's own, trimmed, or a fallback.
    pub fn message(&self) -> String {
        let text = self.stderr.trim();
        if text.is_empty() {
            self.stdout.trim().to_string()
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Extra environment for git calls made on this thread, so tests can isolate themselves from the
    /// machine's own git config (a signing key, say) without changing the whole test process.
    pub(crate) static TEST_ENV: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs git in `repo` with a time limit. Output is read on separate threads so a large answer cannot
/// fill a pipe and stall the child.
fn run(repo: &Path, args: &[&str], timeout: Duration, read_only: bool) -> Result<GitOut> {
    // `core.quotepath=off` keeps non-ASCII names readable; the rest of the config is the machine's own.
    run_program("git", &["-c", "core.quotepath=off"], repo, args, timeout, read_only)
}

/// `gh` (the GitHub CLI) in `repo`: never prompts, never colours, with the same time limit and thread-read
/// treatment as git.
pub fn gh(repo: &Path, args: &[&str], timeout: Duration) -> Result<GitOut> {
    run_program("gh", &[], repo, args, timeout, false)
}

fn run_program(
    program: &str,
    prefix: &[&str],
    repo: &Path,
    args: &[&str],
    timeout: Duration,
    read_only: bool,
) -> Result<GitOut> {
    let mut cmd = Command::new(program);
    cmd.args(prefix)
        .args(args)
        .current_dir(repo)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if read_only {
        // A status taken while someone else commits must not take the index lock away from them.
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
    }
    if std::env::var_os("GIT_ASKPASS").is_none() {
        cmd.env("GIT_ASKPASS", "true");
    }
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=15");
    }
    #[cfg(test)]
    TEST_ENV.with(|env| {
        for (k, v) in env.borrow().iter() {
            cmd.env(k, v);
        }
    });

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            TendrilError::Git(format!("`{program}` is not installed on the machine the daemon runs on."))
        } else {
            TendrilError::Git(format!("Could not run {program}: {e}"))
        }
    })?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(TendrilError::Git(format!("git failed: {e}"))),
        }
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).to_string();
    match status {
        Some(status) => Ok(GitOut { code: status.code().unwrap_or(-1), stdout, stderr }),
        None => Err(TendrilError::Git(format!(
            "{program} {} took longer than {} seconds and was stopped. If it needs a password or an SSH key, \
             set that up on the machine the daemon runs on.",
            args.first().copied().unwrap_or(""),
            timeout.as_secs()
        ))),
    }
}

/// A read: takes no locks, short time limit.
pub fn git_read(repo: &Path, args: &[&str]) -> Result<GitOut> {
    run(repo, args, READ_TIMEOUT, true)
}

/// A change to the repository.
pub fn git_write(repo: &Path, args: &[&str]) -> Result<GitOut> {
    run(repo, args, WRITE_TIMEOUT, false)
}

/// Talks to a remote: fetch, pull, push.
pub fn git_network(repo: &Path, args: &[&str]) -> Result<GitOut> {
    run(repo, args, NETWORK_TIMEOUT, false)
}

/// Like [`git_read`] but a non-zero exit is an error carrying git's own message.
pub fn git_read_ok(repo: &Path, args: &[&str]) -> Result<String> {
    let out = git_read(repo, args)?;
    if out.ok() {
        Ok(out.stdout)
    } else {
        Err(TendrilError::Git(out.message()))
    }
}

/// The repository's real git directory (a linked worktree keeps its own).
pub fn git_dir(repo: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(git_read_ok(repo, &["rev-parse", "--absolute-git-dir"])?.trim()))
}

/// A branch, tag or remote-branch name that is safe to hand to git as a value: not an option, a legal ref
/// name, no surprises.
pub fn validate_ref_name(name: &str) -> Result<()> {
    let bad = |why: &str| Err(TendrilError::Validation(format!("'{name}' is not a valid name: {why}")));
    if name.trim().is_empty() {
        return bad("it is empty");
    }
    if name.starts_with('-') {
        return bad("it starts with '-'");
    }
    if name.chars().any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c)) {
        return bad("it has a space or one of ~ ^ : ? * [ \\");
    }
    if name.contains("..") || name.contains("@{") || name.ends_with('/') || name.ends_with(".lock") || name == "@" {
        return bad("git does not allow that form");
    }
    if name.starts_with('/') || name.contains("//") || name.split('/').any(|p| p.starts_with('.')) {
        return bad("git does not allow that form");
    }
    Ok(())
}

/// Resolves a revision (hash, branch, tag, `HEAD~2`) to a full commit hash, or says it does not exist.
pub fn resolve_commit(repo: &Path, rev: &str) -> Result<String> {
    if rev.trim().is_empty() || rev.starts_with('-') || rev.chars().any(|c| c.is_control()) {
        return Err(TendrilError::Validation(format!("'{rev}' is not a revision")));
    }
    let spec = format!("{rev}^{{commit}}");
    let out = git_read(repo, &["rev-parse", "--verify", "--quiet", &spec])?;
    if out.ok() && !out.stdout.trim().is_empty() {
        Ok(out.stdout.trim().to_string())
    } else {
        Err(TendrilError::Validation(format!("'{rev}' is not a commit in this repository")))
    }
}

/// A path from a client: relative, no `..`, no NUL. Returns it unchanged (git takes it relative to the
/// repository root, after `--`).
pub fn validate_rel_path(path: &str) -> Result<()> {
    if path.is_empty() || path.contains('\0') {
        return Err(TendrilError::Validation("A file path is empty".into()));
    }
    let p = Path::new(path);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_))) {
        return Err(TendrilError::Validation(format!("'{path}' is not a path inside the repository")));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;

    /// A scratch repository with a fixed identity, no signing, and the machine's git config ignored.
    pub struct Scratch {
        pub dir: PathBuf,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    pub fn isolate() {
        TEST_ENV.with(|env| {
            *env.borrow_mut() = vec![
                ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                ("GIT_CONFIG_SYSTEM".into(), "/dev/null".into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GIT_AUTHOR_NAME".into(), "Tester".into()),
                ("GIT_AUTHOR_EMAIL".into(), "tester@example.com".into()),
                ("GIT_COMMITTER_NAME".into(), "Tester".into()),
                ("GIT_COMMITTER_EMAIL".into(), "tester@example.com".into()),
                ("GIT_CONFIG_COUNT".into(), "2".into()),
                ("GIT_CONFIG_KEY_0".into(), "commit.gpgsign".into()),
                ("GIT_CONFIG_VALUE_0".into(), "false".into()),
                ("GIT_CONFIG_KEY_1".into(), "init.defaultBranch".into()),
                ("GIT_CONFIG_VALUE_1".into(), "main".into()),
            ];
        });
    }

    impl Scratch {
        pub fn new() -> Self {
            isolate();
            let dir = std::env::temp_dir().join(format!("tendril-gitws-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&dir).unwrap();
            let s = Self { dir };
            s.git(&["init", "-q"]);
            s
        }

        pub fn git(&self, args: &[&str]) -> String {
            let out = git_write(&self.dir, args).unwrap();
            assert!(out.ok(), "git {:?} failed: {}", args, out.message());
            out.stdout
        }

        pub fn write(&self, rel: &str, content: &str) {
            let path = self.dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }

        /// Writes a file and commits it, returning the new commit hash.
        pub fn commit(&self, rel: &str, content: &str, message: &str) -> String {
            self.write(rel, content);
            self.git(&["add", "--", rel]);
            self.git(&["commit", "-q", "-m", message]);
            self.git(&["rev-parse", "HEAD"]).trim().to_string()
        }
    }

    #[test]
    fn names_revisions_and_paths_are_validated_before_git_sees_them() {
        for good in ["main", "feature/login-form", "release-1.2", "tendril/00042-FixThing"] {
            assert!(validate_ref_name(good).is_ok(), "{good}");
        }
        for bad in ["", "-D", "--force", "a b", "a..b", "a~1", "a:b", "x.lock", "/a", "a//b", ".hidden/x", "a/", "@", "a@{1}"] {
            assert!(validate_ref_name(bad).is_err(), "{bad:?} must be refused");
        }
        for good in ["src/lib.rs", "a b/c.txt"] {
            assert!(validate_rel_path(good).is_ok(), "{good}");
        }
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a\0b"] {
            assert!(validate_rel_path(bad).is_err(), "{bad:?} must be refused");
        }

        let repo = Scratch::new();
        let head = repo.commit("a.txt", "one\n", "first");
        assert_eq!(resolve_commit(&repo.dir, "HEAD").unwrap(), head);
        assert_eq!(resolve_commit(&repo.dir, "main").unwrap(), head);
        assert!(resolve_commit(&repo.dir, "--output=/tmp/x").is_err());
        assert!(resolve_commit(&repo.dir, "nope").is_err());
    }

    #[test]
    fn a_command_that_runs_too_long_is_stopped_with_a_useful_message() {
        let repo = Scratch::new();
        let err = run(&repo.dir, &["-c", "alias.nap=!sleep 5", "nap"], Duration::from_millis(200), true).unwrap_err();
        assert!(err.to_string().contains("took longer than"), "{err}");
    }
}
