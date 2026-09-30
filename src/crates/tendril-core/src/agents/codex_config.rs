//! What Codex's own `config.toml` says it runs, when that is not OpenAI.
//!
//! Codex can be pointed at any OpenAI-compatible endpoint (Ollama, LM Studio, vLLM, a proxy) with
//! `model_provider` and a `[model_providers.<id>]` table. Tendril's catalog and tier defaults only
//! know OpenAI's model ids, so on such a machine they would pass `--model gpt-…` and override the
//! operator's local model. This reads the file on the daemon's host - where Codex runs - so the
//! catalog can offer the configured model and the tier defaults can step aside.
//!
//! Only the active selection is read: top-level `model` / `model_provider`, overlaid by the
//! `[profiles.<profile>]` table when a top-level `profile` names one. `openai` (or no provider) is
//! Codex's default and changes nothing.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCustomProvider {
    /// The `model_provider` id, e.g. `ollama`.
    pub provider: String,
    /// `[model_providers.<id>].name` when set, for labels; the id otherwise.
    pub provider_name: String,
    /// The configured `model`, if the file names one.
    pub model: Option<String>,
}

/// `$CODEX_HOME/config.toml`, else `~/.codex/config.toml`.
pub fn codex_config_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        if !home.trim().is_empty() {
            return Some(PathBuf::from(home).join("config.toml"));
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(PathBuf::from(home).join(".codex").join("config.toml"))
}

/// The non-OpenAI provider Codex is configured for on this machine, if any.
pub fn codex_custom_provider() -> Option<CodexCustomProvider> {
    let text = std::fs::read_to_string(codex_config_path()?).ok()?;
    parse_custom_provider(&text)
}

fn parse_custom_provider(text: &str) -> Option<CodexCustomProvider> {
    let doc: toml::Table = text.parse().ok()?;
    let str_at = |table: &toml::Table, key: &str| {
        table
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let mut provider = str_at(&doc, "model_provider");
    let mut model = str_at(&doc, "model");
    if let Some(profile) = str_at(&doc, "profile") {
        if let Some(table) = doc
            .get("profiles")
            .and_then(|p| p.get(&profile))
            .and_then(|p| p.as_table())
        {
            provider = str_at(table, "model_provider").or(provider);
            model = str_at(table, "model").or(model);
        }
    }

    let provider = provider?;
    if provider.eq_ignore_ascii_case("openai") {
        return None;
    }
    let provider_name = doc
        .get("model_providers")
        .and_then(|p| p.get(&provider))
        .and_then(|p| p.as_table())
        .and_then(|t| str_at(t, "name"))
        .unwrap_or_else(|| provider.clone());
    Some(CodexCustomProvider {
        provider,
        provider_name,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_provider_with_its_model() {
        let found = parse_custom_provider(
            r#"
model = "qwen3-coder:30b"
model_provider = "ollama"

[model_providers.ollama]
name = "Ollama"
base_url = "http://localhost:11434/v1"
"#,
        )
        .unwrap();
        assert_eq!(found.provider, "ollama");
        assert_eq!(found.provider_name, "Ollama");
        assert_eq!(found.model.as_deref(), Some("qwen3-coder:30b"));
    }

    #[test]
    fn the_active_profile_overrides_the_top_level() {
        let found = parse_custom_provider(
            r#"
model = "gpt-5.6-sol"
profile = "local"

[profiles.local]
model = "llama3.3"
model_provider = "lmstudio"
"#,
        )
        .unwrap();
        assert_eq!(found.provider, "lmstudio");
        assert_eq!(found.provider_name, "lmstudio");
        assert_eq!(found.model.as_deref(), Some("llama3.3"));
    }

    #[test]
    fn openai_or_no_provider_is_not_custom() {
        assert_eq!(parse_custom_provider("model = \"gpt-5.6-sol\""), None);
        assert_eq!(parse_custom_provider("model_provider = \"openai\""), None);
        assert_eq!(parse_custom_provider("not toml ["), None);
    }
}
