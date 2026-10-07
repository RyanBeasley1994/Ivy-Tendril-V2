//! The Git page's calls: plain JSON passthrough to `/api/git/**`, whose shapes the daemon owns
//! (`routes/git.rs`) and which are not re-declared here. A repository is only ever named by the opaque id
//! the daemon gave it, never a path.

use super::{path_segment, TendrilClient};
use crate::error::BridgeError;
use std::time::Duration;

/// Fetch, pull, push and PR creation talk to a remote; the daemon gives them two minutes.
const NETWORK_OP_TIMEOUT: Duration = Duration::from_secs(150);

impl TendrilClient {
    async fn git_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
        action: &str,
        timeout: Option<Duration>,
    ) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.client.request(method, &url).headers(self.headers());
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let resp = request.send().await?;
        if !resp.status().is_success() {
            Self::expect_success(resp, "GIT_FAILED", action).await?;
            return Err(BridgeError::new("GIT_FAILED", format!("Could not {action}")));
        }
        Ok(resp.json().await?)
    }

    pub async fn git_repos(&self) -> Result<serde_json::Value, BridgeError> {
        self.git_request(reqwest::Method::GET, "/api/git/repos", None, "list the repositories", None).await
    }

    pub async fn git_prs(&self) -> Result<serde_json::Value, BridgeError> {
        self.git_request(reqwest::Method::GET, "/api/git/prs", None, "read the pull requests", Some(NETWORK_OP_TIMEOUT)).await
    }

    pub async fn git_repo(&self, id: &str) -> Result<serde_json::Value, BridgeError> {
        self.git_request(reqwest::Method::GET, &format!("/api/git/repos/{}", path_segment(id)), None, "read the repository", None).await
    }

    pub async fn git_graph(&self, id: &str, limit: u32, branch: Option<&str>) -> Result<serde_json::Value, BridgeError> {
        let mut path = format!("/api/git/repos/{}/graph?limit={limit}", path_segment(id));
        if let Some(b) = branch {
            path.push_str(&format!("&branch={}", super::urlencoding(b)));
        }
        self.git_request(reqwest::Method::GET, &path, None, "read the commit graph", None).await
    }

    pub async fn git_commit(&self, id: &str, hash: &str) -> Result<serde_json::Value, BridgeError> {
        self.git_request(
            reqwest::Method::GET,
            &format!("/api/git/repos/{}/commits/{}", path_segment(id), path_segment(hash)),
            None,
            "read the commit",
            None,
        )
        .await
    }

    pub async fn git_diff(&self, id: &str, request: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        self.git_request(reqwest::Method::POST, &format!("/api/git/repos/{}/diff", path_segment(id)), Some(request), "read the diff", None).await
    }

    pub async fn git_history(&self, id: &str, path: &str, limit: u32) -> Result<serde_json::Value, BridgeError> {
        let uri = format!("/api/git/repos/{}/history?path={}&limit={limit}", path_segment(id), super::urlencoding(path));
        self.git_request(reqwest::Method::GET, &uri, None, "read the file's history", None).await
    }

    pub async fn git_op(&self, id: &str, op: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        self.git_request(reqwest::Method::POST, &format!("/api/git/repos/{}/op", path_segment(id)), Some(op), "run the git operation", Some(NETWORK_OP_TIMEOUT)).await
    }

    pub async fn git_pr_prefill(&self, id: &str, head: &str, base: &str) -> Result<serde_json::Value, BridgeError> {
        let uri = format!(
            "/api/git/repos/{}/pr-prefill?head={}&base={}",
            path_segment(id),
            super::urlencoding(head),
            super::urlencoding(base)
        );
        self.git_request(reqwest::Method::GET, &uri, None, "prepare the pull request", None).await
    }

    pub async fn git_create_pr(&self, id: &str, request: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        self.git_request(reqwest::Method::POST, &format!("/api/git/repos/{}/pr", path_segment(id)), Some(request), "create the pull request", Some(NETWORK_OP_TIMEOUT)).await
    }
}
