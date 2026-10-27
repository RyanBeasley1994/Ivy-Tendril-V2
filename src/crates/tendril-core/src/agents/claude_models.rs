//! The models the installed Claude Code CLI itself offers.
//!
//! [`super::catalog`]'s `CLAUDE_MODELS` is declared by hand, so it is out of date the day Anthropic
//! ships a model: Sonnet 5.5 and Haiku 5.5 were launchable for weeks before anyone added a row. The
//! CLI already knows. Its stream-json `initialize` handshake (what the Agent SDK's
//! `supportedModels()` reads) answers with every model the signed-in account can run, each with its
//! real id (`resolvedModel`), a display name and the effort levels it accepts. That handshake sends no
//! prompt, so asking costs nothing but a process start.
//!
//! [`harness_models`] asks once and keeps the answer for [`CACHE_TTL`]; a CLI that is missing, signed
//! out or too old to answer leaves the declared list as it is.

use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::catalog::{effort_options, AgentOption, ModelOption};

/// How long one answer is trusted. New models arrive on a CLI update, not mid-session.
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
/// Retry sooner after a failure, so a CLI installed or signed in later is picked up.
const FAILURE_TTL: Duration = Duration::from_secs(2 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// One entry of the CLI's `initialize` response's `models`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessModel {
    /// What `--model` takes: an alias (`sonnet`) or a full id.
    pub value: String,
    /// The id the alias runs today (`claude-sonnet-5-5`).
    #[serde(default)]
    pub resolved_model: Option<String>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub supported_effort_levels: Vec<String>,
}

impl HarnessModel {
    fn id(&self) -> &str {
        self.resolved_model.as_deref().filter(|m| !m.is_empty()).unwrap_or(&self.value)
    }
}

static CACHE: Mutex<Option<(Instant, Option<Vec<HarnessModel>>)>> = Mutex::new(None);

/// The CLI's models, from cache when fresh. `None` when the CLI could not be asked.
pub async fn harness_models() -> Option<Vec<HarnessModel>> {
    if let Some((at, cached)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let ttl = if cached.is_some() { CACHE_TTL } else { FAILURE_TTL };
        if at.elapsed() < ttl {
            return cached.clone();
        }
    }
    let fresh = probe(&resolve_claude_binary()).await;
    if fresh.is_none() {
        tracing::debug!("could not read Claude Code's model list; keeping the declared catalogue");
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), fresh.clone()));
    fresh
}

/// `claude` on `PATH`, else where its installers put it: a GUI- or launchd-started daemon often has
/// neither on its `PATH`.
fn resolve_claude_binary() -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
    let file = if cfg!(windows) { "claude.exe" } else { "claude" };
    if let Some(path) = std::env::var_os("PATH") {
        if let Some(found) = std::env::split_paths(&path).map(|d| d.join(file)).find(|p| p.is_file()) {
            return found.to_string_lossy().to_string();
        }
    }
    if let Some(home) = home {
        for candidate in [
            Path::new(&home).join(".local").join("bin").join(file),
            Path::new(&home).join(".claude").join("local").join(file),
        ] {
            if candidate.is_file() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    "claude".to_string()
}

/// Starts the CLI in stream-json mode, sends `initialize`, and reads until its control response.
async fn probe(binary: &str) -> Option<Vec<HarnessModel>> {
    let mut child = tokio::process::Command::new(binary)
        .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let request = r#"{"type":"control_request","request_id":"tendril-models","request":{"subtype":"initialize"}}"#;
    stdin.write_all(format!("{request}\n").as_bytes()).await.ok()?;
    stdin.flush().await.ok()?;

    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(models) = parse_initialize_response(&line) {
                return Some(models);
            }
        }
        None
    };
    let models = tokio::time::timeout(PROBE_TIMEOUT, read).await.ok().flatten();
    drop(stdin);
    let _ = child.kill().await;
    models.filter(|m| !m.is_empty())
}

/// The `models` of an `initialize` control response line, or `None` for any other line.
fn parse_initialize_response(line: &str) -> Option<Vec<HarnessModel>> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value["type"] != "control_response" {
        return None;
    }
    serde_json::from_value(value["response"]["response"]["models"].clone()).ok()
}

/// Puts the CLI's models at the head of the Claude row, in the CLI's order, with its names and effort
/// ladders. Declared rows the CLI did not mention stay after them, so a model a config already names
/// is still offered. The CLI's `default` becomes the agent's default.
pub fn apply_claude_harness_models(agents: &mut [AgentOption], models: &[HarnessModel]) {
    let Some(claude) = agents.iter_mut().find(|a| a.id == "claude") else {
        return;
    };
    let mut rows: Vec<ModelOption> = Vec::new();
    for m in models {
        if m.value == "default" {
            continue;
        }
        let id = m.id().to_string();
        if rows.iter().any(|r| r.id == id) {
            continue;
        }
        let name = m.display_name.trim();
        let display_name = match name {
            "" => id.clone(),
            n if n.starts_with("Claude") => n.to_string(),
            n => format!("Claude {n}"),
        };
        let levels: Vec<&str> = m.supported_effort_levels.iter().map(String::as_str).collect();
        let efforts = effort_options(&levels);
        rows.push(ModelOption { id, display_name, efforts });
    }
    if rows.is_empty() {
        return;
    }
    let declared = std::mem::take(&mut claude.models);
    let leftover: Vec<ModelOption> = declared.into_iter().filter(|d| !rows.iter().any(|r| r.id == d.id)).collect();
    rows.extend(leftover);
    claude.models = rows;
    if let Some(default) = models.iter().find(|m| m.value == "default") {
        claude.default_model = default.id().to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::catalog::all_agents;

    const RESPONSE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"tendril-models","response":{"models":[
        {"value":"default","resolvedModel":"claude-opus-5-5","displayName":"Default (recommended)","supportedEffortLevels":["low","high"]},
        {"value":"opus","resolvedModel":"claude-opus-5-5","displayName":"Opus 5.5","supportedEffortLevels":["low","medium","high","xhigh","max"]},
        {"value":"sonnet","resolvedModel":"claude-sonnet-5-5","displayName":"Sonnet 5.5","supportedEffortLevels":["low","medium","high","xhigh","max"]},
        {"value":"claude-haiku-4-5-20251001","resolvedModel":"claude-haiku-4-5-20251001","displayName":"Haiku 4.5"}
    ]}}}"#;

    #[test]
    fn only_the_initialize_response_is_read() {
        assert!(parse_initialize_response(r#"{"type":"system","subtype":"hook_started"}"#).is_none());
        let models = parse_initialize_response(&RESPONSE.replace('\n', "")).unwrap();
        assert_eq!(models.len(), 4);
        assert_eq!(models[2].id(), "claude-sonnet-5-5");
    }

    #[test]
    fn harness_models_lead_the_claude_row_and_set_its_default() {
        let models = parse_initialize_response(&RESPONSE.replace('\n', "")).unwrap();
        let mut agents = all_agents();
        apply_claude_harness_models(&mut agents, &models);
        let claude = agents.iter().find(|a| a.id == "claude").unwrap();
        let ids: Vec<&str> = claude.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(&ids[..3], &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5-20251001"]);
        assert_eq!(ids.iter().filter(|id| **id == "claude-opus-5-5").count(), 1, "{ids:?}");
        assert!(ids.contains(&"claude-sonnet-5"), "declared rows stay: {ids:?}");
        assert_eq!(claude.models[1].display_name, "Claude Sonnet 5.5");
        assert_eq!(claude.models[1].efforts.len(), 6, "Default plus the CLI's five");
        assert!(claude.models[2].efforts.is_empty());
        assert_eq!(claude.default_model, "claude-opus-5-5");
    }

    /// Asks the real CLI. Opt-in: needs `claude` installed and signed in.
    #[tokio::test]
    #[ignore]
    async fn probes_the_installed_cli() {
        let models = harness_models().await.expect("the installed claude answered initialize");
        let ids: Vec<&str> = models.iter().map(HarnessModel::id).collect();
        println!("{ids:?}");
        assert!(!ids.is_empty());
    }
}
