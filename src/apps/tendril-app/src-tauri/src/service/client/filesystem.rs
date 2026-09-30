//! The folder browser's listing of the daemon host's disk (`GET /api/fs/directories`).

use super::TendrilClient;
use crate::error::BridgeError;

impl TendrilClient {
    /// Handed back as the daemon's JSON: the webview owns the shape, and nothing here reads it.
    pub async fn list_directories(
        &self,
        path: Option<&str>,
        show_hidden: bool,
    ) -> Result<serde_json::Value, BridgeError> {
        let url = format!("{}/api/fs/directories", self.base_url);
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(path) = path {
            query.push(("path", path));
        }
        if show_hidden {
            query.push(("showHidden", "true"));
        }
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&query)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(BridgeError::new(
                "LIST_DIRECTORIES_FAILED",
                Self::service_error(resp)
                    .await
                    .unwrap_or_else(|| format!("Failed to list directories ({status})")),
            ));
        }

        Ok(resp.json().await?)
    }
}
