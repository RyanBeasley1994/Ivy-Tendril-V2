//! The model aliases Kimi Code's own `config.toml` declares.
//!
//! `kimi -m` takes an alias, not a provider model id, and the aliases are whatever the operator's
//! `[models."<alias>"]` tables say - `kimi login` writes the `kimi-code/*` ones, and anything else is
//! hand-added. The CLI has no subcommand that lists them (`/models` is an in-session refresh), so the
//! file on the daemon's host, where `kimi` runs, is the only listing there is.

use std::path::PathBuf;

use crate::agents::provider_models::{
    DiscoveredModel, ModelProviderKind, ProfileDefaults, ProviderModelsOutcome,
};

/// The suffix Kimi gives the faster, pricier variant of a model alias
/// (`kimi-code/kimi-for-coding-highspeed`). The settings pane's fast-mode switch swaps on it.
pub const KIMI_FAST_SUFFIX: &str = "-highspeed";

/// `$KIMI_CODE_HOME/config.toml`, else `~/.kimi-code/config.toml`.
pub fn kimi_config_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("KIMI_CODE_HOME") {
        if !home.trim().is_empty() {
            return Some(PathBuf::from(home).join("config.toml"));
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(PathBuf::from(home).join(".kimi-code").join("config.toml"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiModels {
    /// Every `[models."<alias>"]` table, keyed by alias.
    pub models: Vec<DiscoveredModel>,
    /// Top-level `default_model`, when it names one of `models`.
    pub default_model: Option<String>,
}

/// What `POST /api/agents/models` answers for `kimi`: the aliases on this machine, or why there are
/// none. No tier is preselected beyond `default_model`, because which alias suits which tier is the
/// operator's call - and an unset tier already runs `default_model`.
pub fn discover_kimi_models() -> ProviderModelsOutcome {
    let Some(path) = kimi_config_path() else {
        return missing("Could not locate the home directory to find Kimi Code's config.toml.");
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            return missing(&format!(
                "No Kimi Code config at {}. Run `kimi login` to create it.",
                path.display()
            ))
        }
    };
    let Some(parsed) = parse_models(&text) else {
        return missing(&format!("Could not parse {}.", path.display()));
    };
    if parsed.models.is_empty() {
        return missing(&format!(
            "{} declares no models. Run `kimi login` to add Kimi's own.",
            path.display()
        ));
    }

    let default = parsed
        .default_model
        .clone()
        .unwrap_or_else(|| parsed.models[0].id.clone());
    ProviderModelsOutcome::Models {
        models: parsed.models,
        defaults: ProfileDefaults {
            deep: default.clone(),
            balanced: default.clone(),
            quick: default,
        },
        provider: ModelProviderKind::Generic,
    }
}

/// Reported on the key field's channel: for Kimi the "credential" is the `kimi login` that writes the
/// file, so a missing file is the same failure a missing key is elsewhere.
fn missing(message: &str) -> ProviderModelsOutcome {
    ProviderModelsOutcome::ApiKeyError {
        message: message.to_string(),
    }
}

fn parse_models(text: &str) -> Option<KimiModels> {
    let doc: toml::Table = text.parse().ok()?;
    let models: Vec<DiscoveredModel> = doc
        .get("models")
        .and_then(|m| m.as_table())
        .map(|table| {
            table
                .iter()
                .filter_map(|(alias, entry)| {
                    let entry = entry.as_table()?;
                    let label = ["display_name", "model"]
                        .iter()
                        .find_map(|key| entry.get(*key).and_then(|v| v.as_str()))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(alias);
                    Some(DiscoveredModel {
                        id: alias.clone(),
                        display_name: label.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let default_model = doc
        .get("default_model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| models.iter().any(|m| m.id == *id))
        .map(str::to_string);
    Some(KimiModels {
        models,
        default_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
default_model = "kimi-code/kimi-for-coding"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
api_key = ""

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
display_name = "Kimi for Coding"

[models."kimi-code/kimi-for-coding-highspeed"]
provider = "managed:kimi-code"
model = "kimi-for-coding-highspeed"
max_context_size = 262144
"#;

    #[test]
    fn every_alias_is_listed_with_its_label() {
        let parsed = parse_models(SAMPLE).unwrap();
        assert_eq!(
            parsed.models,
            vec![
                DiscoveredModel {
                    id: "kimi-code/kimi-for-coding".into(),
                    display_name: "Kimi for Coding".into(),
                },
                // No `display_name`: the provider model id is the next best label.
                DiscoveredModel {
                    id: "kimi-code/kimi-for-coding-highspeed".into(),
                    display_name: "kimi-for-coding-highspeed".into(),
                },
            ]
        );
        assert_eq!(
            parsed.default_model.as_deref(),
            Some("kimi-code/kimi-for-coding")
        );
    }

    #[test]
    fn a_default_naming_no_declared_alias_is_dropped() {
        let parsed = parse_models("default_model = \"gone\"\n[models.a]\nmodel = \"a\"\n").unwrap();
        assert_eq!(parsed.default_model, None);
        assert_eq!(parsed.models.len(), 1);
    }

    #[test]
    fn a_file_without_models_parses_to_none_listed() {
        assert!(parse_models("default_model = \"\"\n")
            .unwrap()
            .models
            .is_empty());
        assert!(parse_models("not = [valid").is_none());
    }
}
