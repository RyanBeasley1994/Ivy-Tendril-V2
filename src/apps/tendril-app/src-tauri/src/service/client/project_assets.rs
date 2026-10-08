//! A project's memory files and its repo-asset import — the routes behind Settings' project memory
//! table (`EditProjectMemorySheet`) and `ImportRepoAssetsDialog`.
//!
//! Plain JSON passthrough, like the vault calls: the shapes are the daemon's
//! (`routes/projects/memory.rs`, `routes/projects/repo_assets.rs`) and are not re-declared here.

use super::{path_segment, TendrilClient, CLONE_TIMEOUT};
use crate::error::BridgeError;

impl TendrilClient {
    async fn project_assets_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
        failure_code: &str,
        action: &str,
        timeout: Option<std::time::Duration>,
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
            // `expect_success` answers `Err` for every non-2xx status, carrying the daemon's message.
            Self::expect_success(resp, failure_code, action).await?;
            return Err(BridgeError::new(
                failure_code,
                format!("Could not {action}"),
            ));
        }
        Ok(resp.json().await?)
    }

    /// The public API's keys (never their secrets).
    pub async fn list_api_keys(&self) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(reqwest::Method::GET, "/api/api-keys", None, "API_KEYS_FAILED", "list the API keys", None)
            .await
    }

    /// Creates a key. The response holds the secret, which is shown once and never stored in the clear.
    pub async fn create_api_key(&self, name: &str, write: bool) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::POST,
            "/api/api-keys",
            Some(serde_json::json!({ "name": name, "write": write })),
            "API_KEY_CREATE_FAILED",
            "create the API key",
            None,
        )
        .await
    }

    pub async fn revoke_api_key(&self, id: &str) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::DELETE,
            &format!("/api/api-keys/{}", path_segment(id)),
            None,
            "API_KEY_REVOKE_FAILED",
            "revoke the API key",
            None,
        )
        .await
    }

    /// The screenshots and recordings a mission's workers attached, grouped by milestone.
    pub async fn mission_evidence(&self, mission_id: &str) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            &format!("/api/missions/{}/evidence", path_segment(mission_id)),
            None,
            "MISSION_EVIDENCE_FAILED",
            "read the mission's evidence",
            None,
        )
        .await
    }

    /// Every screenshot and recording attached to any of the project's plans.
    pub async fn project_evidence(&self, project_name: &str) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            &format!("/api/projects/{}/evidence", path_segment(project_name)),
            None,
            "PROJECT_EVIDENCE_FAILED",
            "read the project's evidence",
            None,
        )
        .await
    }

    /// The global engines and the rate-limit fallback chain.
    pub async fn get_global_engine(&self) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(reqwest::Method::GET, "/api/engine", None, "GLOBAL_ENGINE_FAILED", "read the global engines", None)
            .await
    }

    /// Sets global roles (`null` clears one) and/or replaces the fallback chain.
    pub async fn set_global_engine(&self, body: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::PUT,
            "/api/engine",
            Some(body),
            "GLOBAL_ENGINE_SET_FAILED",
            "save the global engines",
            None,
        )
        .await
    }

    /// The project's engine per role.
    pub async fn get_project_engine(&self, project_name: &str) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            &format!("/api/projects/{}/engine", path_segment(project_name)),
            None,
            "PROJECT_ENGINE_FAILED",
            "read the project's engines",
            None,
        )
        .await
    }

    /// Sets (or, with `null`, clears) the engine of the roles named in `roles`.
    pub async fn set_project_engine(
        &self,
        project_name: &str,
        roles: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::PUT,
            &format!("/api/projects/{}/engine", path_segment(project_name)),
            Some(serde_json::json!({ "roles": roles })),
            "PROJECT_ENGINE_FAILED",
            "change the project's engines",
            None,
        )
        .await
    }

    /// Whether each project's manager is mid-turn, and when its conversation last moved.
    pub async fn managers_status(&self) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            "/api/projects/managers",
            None,
            "MANAGERS_STATUS_FAILED",
            "read the managers' status",
            None,
        )
        .await
    }

    /// Project name to the owner of its first repo's `origin` remote.
    pub async fn project_owners(&self) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            "/api/projects/owners",
            None,
            "PROJECT_OWNERS_FAILED",
            "read the projects' owners",
            None,
        )
        .await
    }

    /// The project's Factory Manager chat session, created on first use.
    pub async fn get_project_manager(
        &self,
        project_name: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::POST,
            &format!("/api/projects/{}/manager", path_segment(project_name)),
            None,
            "PROJECT_MANAGER_FAILED",
            "open the project's manager",
            None,
        )
        .await
    }

    /// The Docker containers that belong to the project.
    pub async fn project_docker(
        &self,
        project_name: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            &format!("/api/projects/{}/docker", path_segment(project_name)),
            None,
            "PROJECT_DOCKER_FAILED",
            "list the project's containers",
            None,
        )
        .await
    }

    pub async fn list_project_memory(
        &self,
        project_name: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            &format!("/api/projects/{}/memory", path_segment(project_name)),
            None,
            "PROJECT_MEMORY_FAILED",
            "list project memories",
            None,
        )
        .await
    }

    pub async fn get_project_memory(
        &self,
        project_name: &str,
        file_name: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::GET,
            &format!(
                "/api/projects/{}/memory/{}",
                path_segment(project_name),
                path_segment(file_name)
            ),
            None,
            "PROJECT_MEMORY_FAILED",
            "read the memory file",
            None,
        )
        .await
    }

    pub async fn put_project_memory(
        &self,
        project_name: &str,
        file_name: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::PUT,
            &format!(
                "/api/projects/{}/memory/{}",
                path_segment(project_name),
                path_segment(file_name)
            ),
            Some(body),
            "PROJECT_MEMORY_FAILED",
            "save the memory file",
            None,
        )
        .await
    }

    pub async fn delete_project_memory(
        &self,
        project_name: &str,
        file_name: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::DELETE,
            &format!(
                "/api/projects/{}/memory/{}",
                path_segment(project_name),
                path_segment(file_name)
            ),
            None,
            "PROJECT_MEMORY_FAILED",
            "delete the memory file",
            None,
        )
        .await
    }

    /// Scanning may clone a git URL first, so it gets the clone timeout rather than the shared 10s.
    pub async fn scan_repo_assets(
        &self,
        project_name: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::POST,
            &format!(
                "/api/projects/{}/repo-assets/scan",
                path_segment(project_name)
            ),
            Some(body),
            "REPO_ASSETS_FAILED",
            "scan the repository",
            Some(CLONE_TIMEOUT),
        )
        .await
    }

    pub async fn import_repo_assets(
        &self,
        project_name: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.project_assets_request(
            reqwest::Method::POST,
            &format!(
                "/api/projects/{}/repo-assets/import",
                path_segment(project_name)
            ),
            Some(body),
            "REPO_ASSETS_FAILED",
            "import from the repository",
            Some(CLONE_TIMEOUT),
        )
        .await
    }
}
