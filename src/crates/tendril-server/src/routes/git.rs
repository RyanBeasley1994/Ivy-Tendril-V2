//! `/api/git`: the Git page's view of the repositories of the configured projects.
//!
//! * `GET  /api/git/repos` — one summary per project repository (the cards).
//! * `GET  /api/git/prs` — open pull requests per repository, through `gh`, cached for a minute.
//! * `GET  /api/git/repos/:id` — the full workspace overview: status, branches, tags, stashes, worktrees,
//!   which branches belong to a mission or plan, and which missions are running here.
//! * `GET  /api/git/repos/:id/graph?limit=&branch=` — the commit graph, laid out in lanes.
//! * `GET  /api/git/repos/:id/commits/:hash` — one commit's details and files.
//! * `POST /api/git/repos/:id/diff` — one file's diff (commit, staged, unstaged or untracked).
//! * `GET  /api/git/repos/:id/history?path=` — the commits that touched a file.
//! * `POST /api/git/repos/:id/op` — one operation (see `tendril_core::git::workspace::RepoOp`).
//! * `GET  /api/git/repos/:id/pr-prefill?head=&base=` and `POST /api/git/repos/:id/pr` — pull requests.
//!
//! **The client never names a path.** A repository is an opaque id that stands for a repository of a
//! configured project (or one of that repository's own worktrees); anything else is a 404. Operations on
//! one repository run one at a time, so two clicks cannot fight over `index.lock`.

use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tendril_core::config::{expand_variables, load_config};
use tendril_core::error::TendrilError;
use tendril_core::git::workspace as ws;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn tendril_error(e: TendrilError) -> Response {
    match e {
        TendrilError::Validation(m) => error(StatusCode::BAD_REQUEST, m),
        other => error(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

/// FNV-1a: a stable 64-bit hash, so a repository keeps its id from one run of the daemon to the next.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

pub fn repo_id(path: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("{:016x}", fnv(&canonical.to_string_lossy()))
}

#[derive(Clone)]
struct RepoEntry {
    id: String,
    project: String,
    name: String,
    path: PathBuf,
}

fn configured_repos(state: &AppState) -> Vec<RepoEntry> {
    let settings = load_config(&state.config_path).unwrap_or_default();
    let home = state.tendril_home.to_string_lossy().to_string();
    let mut seen = std::collections::HashSet::new();
    let mut repos = Vec::new();
    for project in &settings.projects {
        for repo in &project.repos {
            let path = PathBuf::from(expand_variables(repo.path.trim(), &home));
            if !path.is_dir() {
                continue;
            }
            let id = repo_id(&path);
            if !seen.insert(id.clone()) {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "repo".into());
            repos.push(RepoEntry { id, project: project.name.clone(), name, path });
        }
    }
    repos
}

/// The repository an id stands for: a project repository, or one of its linked worktrees.
fn resolve(state: &AppState, id: &str) -> Result<RepoEntry, Response> {
    let repos = configured_repos(state);
    if let Some(r) = repos.iter().find(|r| r.id == id) {
        return Ok(r.clone());
    }
    for repo in &repos {
        if let Ok(worktrees) = ws::list_worktrees(&repo.path) {
            for w in worktrees.into_iter().filter(|w| !w.main) {
                let path = PathBuf::from(&w.path);
                if repo_id(&path) == id {
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    return Ok(RepoEntry { id: id.to_string(), project: repo.project.clone(), name, path });
                }
            }
        }
    }
    Err(error(StatusCode::NOT_FOUND, "That repository is not one of your projects' repositories."))
}

/// Runs blocking git work off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, TendrilError> + Send + 'static) -> Result<T, Response> {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(tendril_error(e)),
        Err(e) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

fn lock_for(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .entry(id.to_string())
        .or_default()
        .clone()
}

/// `GET /api/git/repos`
pub async fn list_repos(State(state): State<Arc<AppState>>) -> Response {
    let repos = configured_repos(&state);
    let mut set = tokio::task::JoinSet::new();
    for (i, repo) in repos.iter().cloned().enumerate() {
        set.spawn_blocking(move || {
            let summary = ws::read_summary(&repo.path);
            let owner = ws::repo_owner(&repo.path);
            (i, repo, summary, owner)
        });
    }
    let mut rows: Vec<(usize, Value)> = Vec::new();
    while let Some(done) = set.join_next().await {
        let Ok((i, repo, summary, owner)) = done else { continue };
        let (summary, err) = match summary {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e.to_string())),
        };
        rows.push((
            i,
            json!({
                "id": repo.id,
                "project": repo.project,
                "name": repo.name,
                "path": repo.path.to_string_lossy(),
                "owner": owner,
                "summary": summary,
                "error": err,
            }),
        ));
    }
    rows.sort_by_key(|(i, _)| *i);
    Json(json!({ "repos": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>() })).into_response()
}

type PrCache = Mutex<HashMap<String, (Instant, Value)>>;

fn pr_cache() -> &'static PrCache {
    static CACHE: OnceLock<PrCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

const PR_CACHE_TTL: Duration = Duration::from_secs(60);

fn prs_value(path: &std::path::Path) -> Value {
    match ws::list_open_prs(path) {
        Ok(prs) => json!({ "prs": prs, "error": null }),
        Err(e) => json!({ "prs": [], "error": e.to_string() }),
    }
}

/// `GET /api/git/prs` — open pull requests for every repository. `gh` is slow and rate-limited, so each
/// repository's answer is kept for a minute and the cards load it after they have drawn.
pub async fn all_prs(State(state): State<Arc<AppState>>) -> Response {
    let repos = configured_repos(&state);
    let mut set = tokio::task::JoinSet::new();
    for repo in repos {
        let cached = pr_cache().lock().unwrap().get(&repo.id).filter(|(at, _)| at.elapsed() < PR_CACHE_TTL).map(|(_, v)| v.clone());
        set.spawn_blocking(move || {
            let value = cached.unwrap_or_else(|| {
                let v = prs_value(&repo.path);
                pr_cache().lock().unwrap().insert(repo.id.clone(), (Instant::now(), v.clone()));
                v
            });
            (repo.id, value)
        });
    }
    let mut out = serde_json::Map::new();
    while let Some(done) = set.join_next().await {
        if let Ok((id, value)) = done {
            out.insert(id, value);
        }
    }
    Json(json!({ "repos": out })).into_response()
}

/// `GET /api/git/repos/:id/prs`
pub async fn repo_prs(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let path = repo.path.clone();
    match blocking(move || Ok(prs_value(&path))).await {
        Ok(v) => {
            pr_cache().lock().unwrap().insert(repo.id.clone(), (Instant::now(), v.clone()));
            Json(v).into_response()
        }
        Err(r) => r,
    }
}

/// Which branches belong to a mission or a plan, and which missions are working in this project right now.
fn links_and_activity(state: &AppState, project: &str, refs: &[ws::RefInfo]) -> (Value, Value) {
    let mut links = serde_json::Map::new();
    let mut active = Vec::new();
    for f in tendril_core::missions::store::list_missions(&state.mission_driver.paths().missions_dir) {
        if !f.mission.project.eq_ignore_ascii_case(project) {
            continue;
        }
        if let Some(branch) = &f.mission.branch {
            links.insert(
                branch.clone(),
                json!({ "kind": "mission", "id": f.id, "title": f.mission.title, "state": f.mission.state.as_str() }),
            );
        }
        if matches!(
            f.mission.state,
            tendril_core::missions::model::MissionState::Planning
                | tendril_core::missions::model::MissionState::Running
                | tendril_core::missions::model::MissionState::Validating
        ) {
            active.push(json!({
                "id": f.id,
                "title": f.mission.title,
                "state": f.mission.state.as_str(),
                "branch": f.mission.branch,
            }));
        }
    }
    for r in refs.iter().filter(|r| r.kind == ws::RefKind::Local) {
        let Some(folder) = r.name.strip_prefix("tendril/") else { continue };
        if links.contains_key(&r.name) {
            continue;
        }
        if let Ok((plan, _)) = tendril_core::plans::reader::read_plan_yaml(&state.plans_dir.join(folder)) {
            let id = folder.split('-').next().unwrap_or(folder);
            links.insert(r.name.clone(), json!({ "kind": "plan", "id": id, "title": plan.title, "state": plan.state }));
        }
    }
    (Value::Object(links), Value::Array(active))
}

/// `GET /api/git/repos/:id`
pub async fn repo_detail(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let path = repo.path.clone();
    let overview = match blocking(move || ws::read_overview(&path)).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let (links, active) = links_and_activity(&state, &repo.project, &overview.refs);
    let path = repo.path.clone();
    let owner = tokio::task::spawn_blocking(move || ws::repo_owner(&path)).await.ok().flatten();
    // Each added worktree is a repository of its own to open, under an id like any other.
    let worktree_ids: HashMap<String, String> = overview
        .worktrees
        .iter()
        .map(|w| (w.path.clone(), repo_id(std::path::Path::new(&w.path))))
        .collect();
    Json(json!({
        "id": repo.id,
        "project": repo.project,
        "name": repo.name,
        "path": repo.path.to_string_lossy(),
        "owner": owner,
        "overview": overview,
        "links": links,
        "activeWork": active,
        "worktreeIds": worktree_ids,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub branch: Option<String>,
}

/// `GET /api/git/repos/:id/graph`
pub async fn repo_graph(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<GraphQuery>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let limit = q.limit.unwrap_or(300);
    match blocking(move || ws::read_graph(&repo.path, limit, q.branch.as_deref())).await {
        Ok(g) => Json(g).into_response(),
        Err(r) => r,
    }
}

/// `GET /api/git/repos/:id/commits/:hash`
pub async fn repo_commit(State(state): State<Arc<AppState>>, Path((id, hash)): Path<(String, String)>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    match blocking(move || ws::read_commit(&repo.path, &hash)).await {
        Ok(c) => Json(c).into_response(),
        Err(r) => r,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRequest {
    pub target: ws::DiffTarget,
    pub path: String,
    #[serde(default)]
    pub orig_path: Option<String>,
}

/// `POST /api/git/repos/:id/diff`
pub async fn repo_diff(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<DiffRequest>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    match blocking(move || ws::read_file_diff(&repo.path, &req.target, &req.path, req.orig_path.as_deref())).await {
        Ok(d) => Json(d).into_response(),
        Err(r) => r,
    }
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub path: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/git/repos/:id/history?path=`
pub async fn repo_history(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<HistoryQuery>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    match blocking(move || ws::file_history(&repo.path, &q.path, q.limit.unwrap_or(100))).await {
        Ok(h) => Json(json!({ "commits": h })).into_response(),
        Err(r) => r,
    }
}

/// `POST /api/git/repos/:id/op` — runs one operation under the repository's lock and answers with what
/// happened and the repository's summary afterwards.
pub async fn repo_op(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(op): Json<ws::RepoOp>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let lock = lock_for(&repo.id);
    let _guard = lock.lock().await;
    let path = repo.path.clone();
    let result = blocking(move || {
        let outcome = ws::run_op(&path, &op)?;
        let summary = ws::read_summary(&path)?;
        Ok((outcome, summary))
    })
    .await;
    // Whatever changed, the cached pull requests may be stale (a push opens the way to one).
    pr_cache().lock().unwrap().remove(&repo.id);
    match result {
        Ok((outcome, summary)) => Json(json!({ "outcome": outcome, "summary": summary })).into_response(),
        Err(r) => r,
    }
}

#[derive(Debug, Deserialize)]
pub struct PrefillQuery {
    pub head: String,
    pub base: String,
}

/// `GET /api/git/repos/:id/pr-prefill?head=&base=`
pub async fn pr_prefill(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<PrefillQuery>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    match blocking(move || ws::prefill_pr(&repo.path, &q.head, &q.base)).await {
        Ok(p) => Json(p).into_response(),
        Err(r) => r,
    }
}

#[derive(Debug, Deserialize)]
pub struct CreatePrRequest {
    pub head: String,
    pub base: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub draft: bool,
}

/// `POST /api/git/repos/:id/pr`
pub async fn create_pr(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<CreatePrRequest>) -> Response {
    let repo = match resolve(&state, &id) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let lock = lock_for(&repo.id);
    let _guard = lock.lock().await;
    let id = repo.id.clone();
    let result = blocking(move || ws::create_pr_for(&repo.path, &req.head, &req.base, &req.title, &req.body, req.draft)).await;
    pr_cache().lock().unwrap().remove(&id);
    match result {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(r) => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_keeps_the_same_id_and_two_repositories_differ() {
        let a = std::env::temp_dir();
        assert_eq!(repo_id(&a), repo_id(&a));
        assert_ne!(repo_id(&a), repo_id(&a.join("elsewhere")));
        assert_eq!(repo_id(&a).len(), 16);
    }

    #[test]
    fn the_lock_for_a_repository_is_shared_between_callers() {
        let x = lock_for("one");
        let y = lock_for("one");
        let z = lock_for("two");
        assert!(Arc::ptr_eq(&x, &y));
        assert!(!Arc::ptr_eq(&x, &z));
    }
}
