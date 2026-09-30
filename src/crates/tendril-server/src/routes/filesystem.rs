//! `GET /api/fs/directories?path=<abs>` — the folder browser behind "Browse" when picking a project's
//! repository.
//!
//! The desktop app uses the native picker, which is right only when the app and the daemon share a
//! machine. A client reaching the daemon over a tunnel is picking a folder *on the daemon's host*, and
//! nothing on the client's side can show it that disk, so the listing has to come from here.
//!
//! Directories only: the picker's answer is a repository root, and a file name is disclosure the
//! question does not need. It sits on the protected router, so it takes the daemon secret or a session
//! token, and a share visitor's capability token never reaches it (`share_token_allows` is
//! deny-by-default).

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Component, Path, PathBuf};

/// Enough for any real directory, small enough that a `node_modules` cannot become a 10 MB response.
const MAX_ENTRIES: usize = 2000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoriesQuery {
    /// Absolute path to list. Absent means the daemon user's home directory.
    pub path: Option<String>,
    #[serde(default)]
    pub show_hidden: bool,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    /// Holds a `.git` directory, or a `.git` file as a worktree does.
    pub is_git_repo: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub home: Option<String>,
    pub is_git_repo: bool,
    pub entries: Vec<DirectoryEntry>,
    /// More than [`MAX_ENTRIES`] subdirectories; the rest were dropped after sorting.
    pub truncated: bool,
    /// Filesystem roots to jump to: `/` on Unix, the mounted drive letters on Windows.
    pub roots: Vec<String>,
}

pub async fn list_directories(Query(query): Query<DirectoriesQuery>) -> Response {
    let home = tendril_core::config::dirs_home();
    let requested = match query.path.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => match home.clone() {
            Some(h) => h,
            None => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "No path given and no home directory",
                )
            }
        },
    };
    if !requested.is_absolute() {
        return error(StatusCode::BAD_REQUEST, "Path must be absolute");
    }
    let path = normalize(&requested);

    let result = tokio::task::spawn_blocking(move || {
        read_listing(&path, home.as_deref(), query.show_hidden)
    })
    .await;
    match result {
        Ok(Ok(listing)) => Json(listing).into_response(),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            error(StatusCode::NOT_FOUND, "Directory not found")
        }
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            error(StatusCode::FORBIDDEN, "Permission denied")
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn read_listing(
    path: &Path,
    home: Option<&Path>,
    show_hidden: bool,
) -> std::io::Result<DirectoryListing> {
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not a directory",
        ));
    }

    let mut entries: Vec<DirectoryEntry> = std::fs::read_dir(path)?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                return None;
            }
            // `Path::is_dir` follows symlinks, so a linked checkout is listed like a real one; an
            // unreadable or dangling entry is simply not a directory.
            let entry_path = entry.path();
            if !entry_path.is_dir() {
                return None;
            }
            Some(DirectoryEntry {
                is_git_repo: is_git_repo(&entry_path),
                path: entry_path.to_string_lossy().into_owned(),
                name,
            })
        })
        .collect();
    entries.sort_by_key(|e| e.name.to_lowercase());
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);

    Ok(DirectoryListing {
        path: path.to_string_lossy().into_owned(),
        parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
        home: home.map(|h| h.to_string_lossy().into_owned()),
        is_git_repo: is_git_repo(path),
        entries,
        truncated,
        roots: roots(),
    })
}

fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Lexical `.`/`..` resolution. Not `canonicalize`: that would swap a symlinked path the operator
/// recognises for its target, and on Windows add a `\\?\` prefix that is then stored as a repo path.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(windows)]
fn roots() -> Vec<String> {
    (b'A'..=b'Z')
        .map(|letter| format!("{}:\\", letter as char))
        .filter(|root| Path::new(root).exists())
        .collect()
}

#[cfg(not(windows))]
fn roots() -> Vec<String> {
    vec!["/".to_string()]
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}-{}", name, uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_only_directories_sorted_and_flags_git_repos() {
        let root = scratch("tendril-fs-list");
        std::fs::create_dir_all(root.join("zeta/.git")).unwrap();
        std::fs::create_dir_all(root.join("Alpha")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("file.txt"), "x").unwrap();
        // A worktree's `.git` is a file.
        std::fs::create_dir_all(root.join("worktree")).unwrap();
        std::fs::write(root.join("worktree/.git"), "gitdir: elsewhere").unwrap();

        let listing = read_listing(&root, None, false).unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "worktree", "zeta"]);
        let git: Vec<_> = listing.entries.iter().map(|e| e.is_git_repo).collect();
        assert_eq!(git, [false, true, true]);
        assert!(!listing.truncated);
        assert_eq!(
            listing.parent,
            root.parent().map(|p| p.to_string_lossy().into_owned())
        );

        let with_hidden = read_listing(&root, None, true).unwrap();
        assert!(with_hidden.entries.iter().any(|e| e.name == ".hidden"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_file_or_missing_path_is_not_found() {
        let root = scratch("tendril-fs-missing");
        std::fs::write(root.join("file.txt"), "x").unwrap();
        let err = read_listing(&root.join("file.txt"), None, false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let err = read_listing(&root.join("nope"), None, false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn normalizes_dot_segments_lexically() {
        let base = std::env::temp_dir();
        assert_eq!(normalize(&base.join("a/./b/../c")), base.join("a/c"));
    }

    #[tokio::test]
    async fn refuses_a_relative_path() {
        let response = list_directories(Query(DirectoriesQuery {
            path: Some("relative/dir".into()),
            show_hidden: false,
        }))
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
