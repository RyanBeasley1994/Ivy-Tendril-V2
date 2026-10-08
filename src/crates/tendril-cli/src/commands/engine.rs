//! `tendril engine`: the engines that apply to every project (a project's own settings override them),
//! and the fallback chain a rate-limited agent hands over along.

use clap::Args;
use std::path::Path;
use tendril_core::config::read_master;
use tendril_core::http::daemon_client;

#[derive(Args)]
#[command(
    about = "Show or set the global agent for each role, and the rate-limit fallback chain",
    long_about = "Show or set which agent each role runs on for every project that does not set its own. With no flags it prints the current settings. \
A role is `agent`, `agent:model` or `agent:model:effort`, e.g. `--worker claude:opus` or `--judge codex:gpt-5:high`. \
`--fallback` takes the chain in order, e.g. `--fallback codex:local,gemini`: when the agent a piece of work runs on hits a rate limit, the next one takes over instead of the work waiting."
)]
pub struct EngineArgs {
    #[arg(long)]
    pub manager: Option<String>,
    #[arg(long)]
    pub planner: Option<String>,
    #[arg(long)]
    pub worker: Option<String>,
    #[arg(long)]
    pub judge: Option<String>,
    #[arg(long)]
    pub validator: Option<String>,
    #[arg(long, value_delimiter = ',', help = "Put these roles back on the configured default agent, e.g. --reset worker,judge")]
    pub reset: Vec<String>,
    #[arg(long, value_delimiter = ',', help = "The fallback chain, in order, replacing the current one")]
    pub fallback: Option<Vec<String>>,
    #[arg(long = "clear-fallback", conflicts_with = "fallback", help = "Remove the fallback chain")]
    pub clear_fallback: bool,
}

const ROLES: [&str; 5] = ["manager", "planner", "worker", "judge", "validator"];

/// `agent`, `agent:model` or `agent:model:effort` as the JSON the daemon takes.
pub fn agent_json(spec: &str) -> anyhow::Result<serde_json::Value> {
    let mut parts = spec.splitn(3, ':').map(str::trim);
    let agent = parts
        .next()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| anyhow::anyhow!("'{spec}' needs an agent, e.g. claude or claude:opus"))?;
    let mut value = serde_json::json!({ "agent": agent });
    if let Some(model) = parts.next().filter(|m| !m.is_empty()) {
        value["model"] = model.into();
    }
    if let Some(effort) = parts.next().filter(|e| !e.is_empty()) {
        value["effort"] = effort.into();
    }
    Ok(value)
}

/// The request body for what the flags ask for; empty when they ask for nothing (meaning: show).
pub fn request_body(args: &EngineArgs) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut roles = serde_json::Map::new();
    for (role, spec) in [
        ("manager", &args.manager),
        ("planner", &args.planner),
        ("worker", &args.worker),
        ("judge", &args.judge),
        ("validator", &args.validator),
    ] {
        if let Some(spec) = spec {
            roles.insert(role.to_string(), agent_json(spec)?);
        }
    }
    for role in &args.reset {
        let role = role.trim().to_ascii_lowercase();
        if !ROLES.contains(&role.as_str()) {
            anyhow::bail!("'{role}' is not a role: {}", ROLES.join(", "));
        }
        if roles.contains_key(&role) {
            anyhow::bail!("--{role} and --reset {role} contradict each other");
        }
        roles.insert(role, serde_json::Value::Null);
    }
    let mut body = serde_json::Map::new();
    if !roles.is_empty() {
        body.insert("roles".into(), roles.into());
    }
    if let Some(chain) = &args.fallback {
        let items: anyhow::Result<Vec<_>> = chain.iter().map(|s| agent_json(s)).collect();
        body.insert("fallbacks".into(), items?.into());
    } else if args.clear_fallback {
        body.insert("fallbacks".into(), serde_json::json!([]));
    }
    Ok(body)
}

fn describe(value: &serde_json::Value) -> String {
    let Some(agent) = value["agent"].as_str() else { return "default".to_string() };
    let mut text = agent.to_string();
    for extra in ["model", "effort"] {
        if let Some(v) = value[extra].as_str() {
            text.push_str(&format!(" · {v}"));
        }
    }
    text
}

pub async fn handle_engine_command(args: EngineArgs, tendril_home: &Path) -> anyhow::Result<()> {
    let master = read_master(tendril_home).ok_or_else(|| anyhow::anyhow!("The Tendril daemon is not running."))?;
    let url = format!("{}/api/engine", master.base_url());
    let body = request_body(&args)?;
    let client = daemon_client(tendril_home);
    let request = if body.is_empty() { client.get(&url) } else { client.put(&url).json(&body) };
    let resp = request.bearer_auth(&master.secret).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("{} ({})", resp.text().await.unwrap_or_default().trim(), status);
    }
    let current: serde_json::Value = resp.json().await?;
    println!("Global engines (a project's own settings override these):");
    for role in ROLES {
        println!("  {role:<10} {}", describe(&current["roles"][role]));
    }
    match current["fallbacks"].as_array().filter(|a| !a.is_empty()) {
        Some(chain) => {
            let names: Vec<String> = chain.iter().map(describe).collect();
            println!("Rate-limit fallback: {}", names.join("  →  "));
        }
        None => println!("Rate-limit fallback: none (work waits out a limit)"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> EngineArgs {
        EngineArgs { manager: None, planner: None, worker: None, judge: None, validator: None, reset: vec![], fallback: None, clear_fallback: false }
    }

    #[test]
    fn flags_become_roles_and_a_fallback_chain() {
        let body = request_body(&EngineArgs {
            worker: Some("claude:opus".into()),
            reset: vec!["judge".into()],
            fallback: Some(vec!["codex:local".into(), "gemini".into()]),
            ..args()
        })
        .unwrap();
        assert_eq!(body["roles"]["worker"], serde_json::json!({ "agent": "claude", "model": "opus" }));
        assert!(body["roles"]["judge"].is_null());
        assert_eq!(body["fallbacks"][1], serde_json::json!({ "agent": "gemini" }));
    }

    #[test]
    fn no_flags_means_show_and_bad_ones_are_refused() {
        assert!(request_body(&args()).unwrap().is_empty());
        assert!(request_body(&EngineArgs { reset: vec!["boss".into()], ..args() }).is_err());
        assert!(request_body(&EngineArgs { worker: Some("claude".into()), reset: vec!["worker".into()], ..args() }).is_err());
        assert!(request_body(&EngineArgs { worker: Some(":opus".into()), ..args() }).is_err());
        let cleared = request_body(&EngineArgs { clear_fallback: true, ..args() }).unwrap();
        assert_eq!(cleared["fallbacks"], serde_json::json!([]));
    }
}
