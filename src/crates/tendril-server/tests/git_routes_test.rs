//! `/api/git`: the Git page's routes, over a real throwaway repository behind a real project.
//!
//! What these pin is the route layer: that a repository is only ever reached by the opaque id of a
//! project's repository, that every route sits behind the bearer secret, that an operation changes the
//! repository and answers with what it did, and that a stop on conflicts arrives as data the page can act on.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tendril_core::config::{save_config, TendrilSettings};
use tendril_core::models::{ProjectConfig, RepoRef};
use tendril_server::{create_router, AppState};
use tower::ServiceExt;

struct Harness {
    home: PathBuf,
    repo: PathBuf,
    secret: String,
    router: Router,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("run git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// This test binary is its own process, so isolating it from the machine's git config (a signing key,
/// say) is a matter of setting the environment once.
fn isolate_git() {
    for (k, v) in [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_SYSTEM", "/dev/null"),
        ("GIT_AUTHOR_NAME", "Tester"),
        ("GIT_AUTHOR_EMAIL", "tester@example.com"),
        ("GIT_COMMITTER_NAME", "Tester"),
        ("GIT_COMMITTER_EMAIL", "tester@example.com"),
        ("GIT_CONFIG_COUNT", "2"),
        ("GIT_CONFIG_KEY_0", "commit.gpgsign"),
        ("GIT_CONFIG_VALUE_0", "false"),
        ("GIT_CONFIG_KEY_1", "init.defaultBranch"),
        ("GIT_CONFIG_VALUE_1", "main"),
    ] {
        std::env::set_var(k, v);
    }
}

/// A daemon with one project, `demo`, whose single repository has one commit on `main`.
fn harness() -> Harness {
    isolate_git();
    let home = std::env::temp_dir().join(format!("tendril-git-routes-{}", uuid::Uuid::new_v4().simple()));
    let repo = home.join("repos").join("demo-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first commit"]);

    let mut settings = TendrilSettings::default();
    settings.projects.push(ProjectConfig {
        name: "demo".to_string(),
        repos: vec![RepoRef { path: repo.to_string_lossy().to_string(), base_branch: None, extra: Default::default() }],
        ..Default::default()
    });
    std::fs::create_dir_all(home.join("Plans")).unwrap();
    save_config(&home.join("config.yaml"), &settings).expect("write config");

    let secret = tendril_core::config::generate_bearer_secret();
    let state = Arc::new(AppState::with_plans_dir(home.clone(), home.join("Plans"), secret.clone()));
    Harness { home, repo, secret, router: create_router(state) }
}

impl Harness {
    async fn call(&self, method: &str, uri: &str, body: Option<Value>, authorized: bool) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:5010");
        if authorized {
            request = request.header("authorization", format!("Bearer {}", self.secret));
        }
        let body = match body {
            Some(v) => {
                request = request.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let response = self.router.clone().oneshot(request.body(body).unwrap()).await.expect("router responds");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.call("GET", uri, None, true).await
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.call("POST", uri, Some(body), true).await
    }

    /// The id the daemon gave the demo repository.
    async fn repo_id(&self) -> String {
        let (status, body) = self.get("/api/git/repos").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["repos"][0]["id"].as_str().expect("a repo id").to_string()
    }
}

#[tokio::test]
async fn the_cards_list_each_project_repository_with_its_branch_and_state() {
    let h = harness();
    let (status, body) = h.get("/api/git/repos").await;
    assert_eq!(status, StatusCode::OK);
    let repos = body["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 1);
    let card = &repos[0];
    assert_eq!(card["project"], "demo");
    assert_eq!(card["name"], "demo-app");
    assert_eq!(card["summary"]["branch"], "main");
    assert_eq!(card["summary"]["lastCommit"]["subject"], "first commit");
    assert_eq!(card["summary"]["unstaged"], 0);
    assert_eq!(card["error"], Value::Null);

    // Something uncommitted shows on the card.
    std::fs::write(h.repo.join("scratch.txt"), "x\n").unwrap();
    let (_, body) = h.get("/api/git/repos").await;
    assert_eq!(body["repos"][0]["summary"]["untracked"], 1);
}

#[tokio::test]
async fn every_git_route_needs_the_bearer_secret() {
    let h = harness();
    let id = h.repo_id().await;
    for uri in [
        "/api/git/repos".to_string(),
        "/api/git/prs".to_string(),
        format!("/api/git/repos/{id}"),
        format!("/api/git/repos/{id}/graph"),
    ] {
        let (status, _) = h.call("GET", &uri, None, false).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} must not answer without the secret");
    }
    let (status, _) = h.call("POST", &format!("/api/git/repos/{id}/op"), Some(json!({"op": "stageAll"})), false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn only_a_known_repository_id_reaches_git_never_a_path() {
    let h = harness();
    let sibling = h.home.join("not-a-project");
    std::fs::create_dir_all(&sibling).unwrap();
    git(&sibling, &["init", "-q"]);
    for id in ["deadbeefdeadbeef", "..", &sibling.to_string_lossy().replace('/', "%2F"), "%2Fetc"] {
        let (status, _) = h.get(&format!("/api/git/repos/{id}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
        let (status, _) = h.post(&format!("/api/git/repos/{id}/op"), json!({"op": "stageAll"})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
    }
}

#[tokio::test]
async fn a_whole_working_session_stage_commit_branch_merge_and_look_at_the_graph() {
    let h = harness();
    let id = h.repo_id().await;
    let op = format!("/api/git/repos/{id}/op");

    // Work, look at it, stage it, commit it.
    std::fs::write(h.repo.join("README.md"), "hello\nworld\n").unwrap();
    let (_, detail) = h.get(&format!("/api/git/repos/{id}")).await;
    assert_eq!(detail["overview"]["status"]["entries"][0]["path"], "README.md");
    let own_path = detail["overview"]["worktrees"][0]["path"].as_str().unwrap();
    assert_eq!(detail["worktreeIds"][own_path], id.as_str(), "the main worktree is the repository itself");
    let (status, diff) = h
        .post(&format!("/api/git/repos/{id}/diff"), json!({"target": {"kind": "unstaged"}, "path": "README.md"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{diff}");
    assert!(diff["text"].as_str().unwrap().contains("+world"));

    let (status, staged) = h.post(&op, json!({"op": "stage", "paths": ["README.md"]})).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    assert_eq!(staged["summary"]["staged"], 1);
    let (_, done) = h.post(&op, json!({"op": "commit", "message": "add world"})).await;
    assert_eq!(done["outcome"]["ok"], true, "{done}");
    assert_eq!(done["summary"]["lastCommit"]["subject"], "add world");

    // A branch with its own commit, merged back with a merge commit.
    h.post(&op, json!({"op": "createBranch", "name": "feature/x", "checkout": true})).await;
    std::fs::write(h.repo.join("feature.txt"), "f\n").unwrap();
    h.post(&op, json!({"op": "stageAll"})).await;
    h.post(&op, json!({"op": "commit", "message": "feature work"})).await;
    h.post(&op, json!({"op": "checkout", "name": "main"})).await;
    let (_, merged) = h.post(&op, json!({"op": "merge", "branch": "feature/x", "mode": "noFf"})).await;
    assert_eq!(merged["outcome"]["ok"], true, "{merged}");

    let (status, graph) = h.get(&format!("/api/git/repos/{id}/graph?limit=20")).await;
    assert_eq!(status, StatusCode::OK);
    let rows = graph["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["parents"].as_array().unwrap().len(), 2, "the merge commit is on top");
    let hash = rows[0]["hash"].as_str().unwrap();
    let (status, commit) = h.get(&format!("/api/git/repos/{id}/commits/{hash}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(commit["files"][0]["path"], "feature.txt");
    let (_, history) = h.get(&format!("/api/git/repos/{id}/history?path=README.md")).await;
    assert_eq!(history["commits"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_conflicting_merge_answers_with_the_files_and_can_be_aborted_over_the_same_route() {
    let h = harness();
    let id = h.repo_id().await;
    let op = format!("/api/git/repos/{id}/op");
    h.post(&op, json!({"op": "createBranch", "name": "other", "checkout": true})).await;
    std::fs::write(h.repo.join("README.md"), "from other\n").unwrap();
    h.post(&op, json!({"op": "stageAll"})).await;
    h.post(&op, json!({"op": "commit", "message": "other edits readme"})).await;
    h.post(&op, json!({"op": "checkout", "name": "main"})).await;
    std::fs::write(h.repo.join("README.md"), "from main\n").unwrap();
    h.post(&op, json!({"op": "stageAll"})).await;
    h.post(&op, json!({"op": "commit", "message": "main edits readme"})).await;

    let (status, stopped) = h.post(&op, json!({"op": "merge", "branch": "other"})).await;
    assert_eq!(status, StatusCode::OK, "a stop on conflicts is an outcome, not an error: {stopped}");
    assert_eq!(stopped["outcome"]["ok"], false);
    assert_eq!(stopped["outcome"]["conflicts"], json!(["README.md"]));
    assert_eq!(stopped["outcome"]["state"], "merging");
    assert_eq!(stopped["summary"]["conflicted"], 1);

    let (_, aborted) = h.post(&op, json!({"op": "mergeAbort"})).await;
    assert_eq!(aborted["outcome"]["ok"], true);
    assert_eq!(aborted["summary"]["state"], "clean");
}

#[tokio::test]
async fn bad_input_is_refused_with_a_reason_and_leaves_the_repository_alone() {
    let h = harness();
    let id = h.repo_id().await;
    let op = format!("/api/git/repos/{id}/op");
    let (status, body) = h.post(&op, json!({"op": "createBranch", "name": "--all"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not a valid name"));
    let (status, _) = h.post(&op, json!({"op": "stage", "paths": ["../escape"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = h.post(&op, json!({"op": "rm-rf"})).await;
    assert!(status.is_client_error(), "an operation the page does not know is not run");
    let (status, _) = h.get(&format!("/api/git/repos/{id}/graph?branch=--all")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, detail) = h.get(&format!("/api/git/repos/{id}")).await;
    assert_eq!(detail["overview"]["refs"].as_array().unwrap().len(), 1, "only main exists");
}
