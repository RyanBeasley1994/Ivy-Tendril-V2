//! `tendril manager`: what a project's manager uses to keep a promise to look at something later.
//!
//! A manager only runs when something prompts it, so "I'll check CI in twenty minutes" means nothing
//! unless it also schedules the prompt. `wake` is that: the daemon starts a turn in the manager's chat
//! at the time given, with the note as the reason.

use clap::Subcommand;
use std::path::Path;
use tendril_core::config::read_master;
use tendril_core::http::daemon_client;

#[derive(Subcommand)]
pub enum ManagerCommands {
    #[command(about = "Wake the project's manager later, with a note, to check on something")]
    Wake {
        #[arg(long, help = "The project whose manager to wake")]
        project: String,
        #[arg(long = "in", help = "How long from now: 90s, 20m, 2h, or 1h30m")]
        after: String,
        #[arg(long, help = "What to look at when it wakes, e.g. \"PR 46 CI run 123: if red, delegate a fix\"")]
        note: String,
    },

    #[command(
        about = "Show or change which agent each role of a project runs on (planner, worker, judge, validator)",
        long_about = "Show or change which agent each role of a project runs on. With no role flags it prints the current engines. \
A role is `agent`, `agent:model` or `agent:model:effort`, e.g. `--worker claude` or `--judge codex:gpt-5:high`. \
The change applies at once to the manager, to every live mission from its next job, and to new work; a job already running keeps its agent."
    )]
    Engine {
        #[arg(long, help = "The project")]
        project: String,
        #[arg(long)]
        planner: Option<String>,
        #[arg(long)]
        worker: Option<String>,
        #[arg(long)]
        judge: Option<String>,
        #[arg(long)]
        validator: Option<String>,
        #[arg(long, value_delimiter = ',', help = "Put these roles back on the default agent, e.g. --reset worker,judge")]
        reset: Vec<String>,
    },

    #[command(about = "Have the daemon watch a pull request's checks and wake the manager with the result once they have all finished")]
    WatchPr {
        #[arg(long, help = "The project whose manager to wake")]
        project: String,
        #[arg(long, help = "The pull request number")]
        pr: u64,
        #[arg(long, help = "owner/name, when the PR is not in the project's own repository")]
        repo: Option<String>,
    },
}

/// `90s`, `20m`, `2h`, `1h30m`, or a bare number of minutes.
pub fn parse_duration_seconds(input: &str) -> Option<u64> {
    let text = input.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    if let Ok(minutes) = text.parse::<u64>() {
        return Some(minutes * 60);
    }
    let mut total = 0u64;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            let n: u64 = digits.parse().ok()?;
            digits.clear();
            total += match c {
                's' => n,
                'm' => n * 60,
                'h' => n * 3600,
                'd' => n * 86_400,
                _ => return None,
            };
        }
    }
    if !digits.is_empty() || total == 0 {
        return None;
    }
    Some(total)
}

async fn post_to_manager(tendril_home: &Path, project: &str, action: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let master = read_master(tendril_home)
        .ok_or_else(|| anyhow::anyhow!("The Tendril daemon is not running, so there is nothing to wake the manager."))?;
    let url = format!("{}/api/projects/{}/manager/{}", master.base_url(), project.replace(' ', "%20"), action);
    let resp = daemon_client(tendril_home).post(&url).bearer_auth(&master.secret).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("{} ({})", resp.text().await.unwrap_or_default().trim(), status);
    }
    Ok(resp.json().await?)
}

/// The `roles` body for `PUT /api/projects/:name/engine`: each role named becomes the agent given (or
/// `null` to reset it); roles not named are left alone.
pub fn engine_roles(
    planner: Option<&str>,
    worker: Option<&str>,
    judge: Option<&str>,
    validator: Option<&str>,
    reset: &[String],
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut roles = serde_json::Map::new();
    for (role, spec) in [("planner", planner), ("worker", worker), ("judge", judge), ("validator", validator)] {
        let Some(spec) = spec else { continue };
        let mut parts = spec.splitn(3, ':').map(str::trim);
        let agent = parts.next().filter(|a| !a.is_empty()).ok_or_else(|| anyhow::anyhow!("--{role} needs an agent, e.g. claude or claude:opus"))?;
        let mut value = serde_json::json!({ "agent": agent });
        if let Some(model) = parts.next().filter(|m| !m.is_empty()) {
            value["model"] = model.into();
        }
        if let Some(effort) = parts.next().filter(|e| !e.is_empty()) {
            value["effort"] = effort.into();
        }
        roles.insert(role.to_string(), value);
    }
    for role in reset {
        let role = role.trim().to_ascii_lowercase();
        if !["planner", "worker", "judge", "validator"].contains(&role.as_str()) {
            anyhow::bail!("'{role}' is not a role: planner, worker, judge or validator");
        }
        if roles.contains_key(&role) {
            anyhow::bail!("--{role} and --reset {role} contradict each other");
        }
        roles.insert(role, serde_json::Value::Null);
    }
    Ok(roles)
}

pub async fn handle_manager_command(cmd: ManagerCommands, tendril_home: &Path) -> anyhow::Result<()> {
    match cmd {
        ManagerCommands::Engine { project, planner, worker, judge, validator, reset } => {
            let master = read_master(tendril_home)
                .ok_or_else(|| anyhow::anyhow!("The Tendril daemon is not running."))?;
            let url = format!("{}/api/projects/{}/engine", master.base_url(), project.replace(' ', "%20"));
            let roles = engine_roles(planner.as_deref(), worker.as_deref(), judge.as_deref(), validator.as_deref(), &reset)?;
            let client = daemon_client(tendril_home);
            let request = if roles.is_empty() {
                client.get(&url)
            } else {
                client.put(&url).json(&serde_json::json!({ "roles": roles }))
            };
            let resp = request.bearer_auth(&master.secret).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                anyhow::bail!("{} ({})", resp.text().await.unwrap_or_default().trim(), status);
            }
            let body: serde_json::Value = resp.json().await?;
            let current = &body["roles"];
            println!("Engines for {project}:");
            for role in ["planner", "worker", "judge", "validator"] {
                let r = &current[role];
                let text = match r["agent"].as_str() {
                    Some(agent) => {
                        let mut t = agent.to_string();
                        for extra in ["model", "effort"] {
                            if let Some(v) = r[extra].as_str() {
                                t.push_str(&format!(" · {v}"));
                            }
                        }
                        t
                    }
                    None => "default".to_string(),
                };
                println!("  {role:<10} {text}");
            }
            Ok(())
        }
        ManagerCommands::WatchPr { project, pr, repo } => {
            post_to_manager(tendril_home, &project, "watch", serde_json::json!({ "pr": pr, "repo": repo })).await?;
            println!("Watching PR {pr}. The manager is woken once every check has finished.");
            Ok(())
        }
        ManagerCommands::Wake { project, after, note } => {
            let seconds = parse_duration_seconds(&after)
                .ok_or_else(|| anyhow::anyhow!("'{after}' is not a duration: use 90s, 20m, 2h or 1h30m"))?;
            let master = read_master(tendril_home)
                .ok_or_else(|| anyhow::anyhow!("The Tendril daemon is not running, so there is nothing to wake the manager."))?;
            let url = format!(
                "{}/api/projects/{}/manager/wake",
                master.base_url(),
                project.replace(' ', "%20")
            );
            let resp = daemon_client(tendril_home)
                .post(&url)
                .bearer_auth(&master.secret)
                .json(&serde_json::json!({ "afterSeconds": seconds, "note": note }))
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                anyhow::bail!("{} ({})", resp.text().await.unwrap_or_default().trim(), status);
            }
            let wake: serde_json::Value = resp.json().await?;
            println!("The manager will be woken at {}.", wake["dueAt"].as_str().unwrap_or("the scheduled time"));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{engine_roles, parse_duration_seconds};

    #[test]
    fn engine_flags_become_the_roles_body() {
        let roles = engine_roles(None, Some("claude"), Some("codex:gpt-5:high"), None, &["validator".to_string()]).unwrap();
        assert_eq!(roles["worker"], serde_json::json!({ "agent": "claude" }));
        assert_eq!(roles["judge"], serde_json::json!({ "agent": "codex", "model": "gpt-5", "effort": "high" }));
        assert!(roles["validator"].is_null());
        assert!(!roles.contains_key("planner"), "a role not named is left alone");
        assert!(engine_roles(None, None, None, None, &["boss".to_string()]).is_err());
        assert!(engine_roles(None, Some("claude"), None, None, &["worker".to_string()]).is_err());
        assert!(engine_roles(None, Some(":opus"), None, None, &[]).is_err());
        assert!(engine_roles(None, None, None, None, &[]).unwrap().is_empty(), "no flags means show");
    }

    #[test]
    fn parses_the_forms_a_manager_writes() {
        assert_eq!(parse_duration_seconds("90s"), Some(90));
        assert_eq!(parse_duration_seconds("20m"), Some(1200));
        assert_eq!(parse_duration_seconds("2h"), Some(7200));
        assert_eq!(parse_duration_seconds("1h30m"), Some(5400));
        assert_eq!(parse_duration_seconds("45"), Some(2700));
        assert_eq!(parse_duration_seconds("soon"), None);
        assert_eq!(parse_duration_seconds("10x"), None);
        assert_eq!(parse_duration_seconds(""), None);
        assert_eq!(parse_duration_seconds("0m"), None);
    }
}
