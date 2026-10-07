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

pub async fn handle_manager_command(cmd: ManagerCommands, tendril_home: &Path) -> anyhow::Result<()> {
    match cmd {
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
    use super::parse_duration_seconds;

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
