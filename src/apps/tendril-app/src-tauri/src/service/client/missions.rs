//! Missions: goals an AI orchestrator breaks into milestones and runs. The shapes are
//! `tendril_core::missions`'s and pass through as `serde_json::Value`, declared once in the
//! frontend's `api.ts`.

use super::{path_segment, TendrilClient};
use crate::error::BridgeError;

/// The control routes a mission accepts, so a typo in the frontend cannot reach an arbitrary path.
pub const MISSION_ACTIONS: [&str; 7] =
    ["approve", "pause", "resume", "cancel", "complete", "reconcile", "request-changes"];

/// Mission writes are followed by a driver step in the same request, and a step can create the
/// mission branch (`git fetch`) or a milestone plan before it answers - well past the client's 10s
/// default on a slow remote.
const MISSION_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

impl TendrilClient {
    async fn mission_response(
        resp: reqwest::Response,
        code: &str,
        what: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // The daemon answers `{ "error": "..." }`; surface that sentence rather than the JSON.
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or(text);
            return Err(BridgeError::new(code, format!("{what} ({status}): {message}")));
        }
        Ok(resp.json().await?)
    }

    pub async fn list_missions(&self) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}/api/missions", self.base_url);
        let resp = self.client.get(&url).headers(self.headers()).send().await?;
        Self::mission_response(resp, "LIST_MISSIONS_FAILED", "Failed to list missions").await
    }

    pub async fn get_mission(&self, id: &str) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}/api/missions/{}", self.base_url, path_segment(id));
        let resp = self.client.get(&url).headers(self.headers()).send().await?;
        Self::mission_response(resp, "GET_MISSION_FAILED", "Failed to load the mission").await
    }

    pub async fn create_mission(
        &self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}/api/missions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .timeout(MISSION_WRITE_TIMEOUT)
            .json(request)
            .send()
            .await?;
        Self::mission_response(resp, "CREATE_MISSION_FAILED", "Failed to create the mission").await
    }

    pub async fn mission_action(
        &self,
        id: &str,
        action: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, BridgeError> {
        if !MISSION_ACTIONS.contains(&action) {
            return Err(BridgeError::new(
                "INVALID_MISSION_ACTION",
                format!("Unknown mission action '{action}'"),
            ));
        }
        let url = format!("{}/api/missions/{}/{}", self.base_url, path_segment(id), action);
        let empty = serde_json::json!({});
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .timeout(MISSION_WRITE_TIMEOUT)
            .json(body.unwrap_or(&empty))
            .send()
            .await?;
        Self::mission_response(resp, "MISSION_ACTION_FAILED", &format!("Failed to {action} the mission"))
            .await
    }

    /// `POST /api/config/branch-preview`: the branch names the given `git:` settings would produce.
    pub async fn preview_branch_names(
        &self,
        git: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}/api/config/branch-preview", self.base_url);
        let resp = self.client.post(&url).headers(self.headers()).json(git).send().await?;
        Self::mission_response(resp, "BRANCH_PREVIEW_FAILED", "Failed to preview branch names").await
    }

    pub async fn put_mission(
        &self,
        id: &str,
        section: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}/api/missions/{}/{}", self.base_url, path_segment(id), section);
        let resp = self
            .client
            .put(&url)
            .headers(self.headers())
            .timeout(MISSION_WRITE_TIMEOUT)
            .json(body)
            .send()
            .await?;
        Self::mission_response(resp, "UPDATE_MISSION_FAILED", "Failed to update the mission").await
    }
}
