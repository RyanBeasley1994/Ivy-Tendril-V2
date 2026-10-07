//! `tendril push`: where the daemon sends its "needs you" and "work complete" pushes, and the morning
//! briefing. It is an ntfy topic URL (the public server or your own); install the ntfy app on your phone
//! and subscribe to the same topic. Settings live in `push.json` in the Tendril home, which the daemon
//! re-reads every few seconds, so no restart is needed.

use clap::Subcommand;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum PushCommands {
    #[command(about = "Set the ntfy topic URL pushes go to, e.g. https://ntfy.sh/my-private-topic")]
    Set {
        url: String,
        #[arg(long, help = "A bearer token, for a protected topic")]
        token: Option<String>,
    },

    #[command(about = "Send a morning briefing from each active manager at a set time")]
    Briefing {
        #[arg(long, help = "Local time, HH:MM, e.g. 08:00", conflicts_with = "off")]
        at: Option<String>,
        #[arg(long = "utc-offset-minutes", default_value_t = 0, allow_hyphen_values = true, help = "Minutes east of UTC for that clock: the daemon's machine is usually on UTC (London summer time is 60)")]
        utc_offset_minutes: i32,
        #[arg(long, help = "Stop sending the briefing")]
        off: bool,
    },

    #[command(about = "Send a test push")]
    Test,

    #[command(about = "Stop all pushes (keeps nothing)")]
    Off,

    #[command(about = "Show what is set")]
    Status,
}

fn path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("push.json")
}

fn read(tendril_home: &Path) -> Value {
    std::fs::read_to_string(path(tendril_home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({}))
}

fn write(tendril_home: &Path, value: &Value) -> anyhow::Result<()> {
    std::fs::create_dir_all(tendril_home)?;
    std::fs::write(path(tendril_home), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

/// `HH:MM` as a clock time, or an error naming what was wrong.
pub fn validate_time(at: &str) -> anyhow::Result<String> {
    let (h, m) = at.split_once(':').ok_or_else(|| anyhow::anyhow!("'{at}' is not a time: use HH:MM, e.g. 08:00"))?;
    let (h, m): (u32, u32) = (
        h.trim().parse().map_err(|_| anyhow::anyhow!("'{at}' is not a time: use HH:MM, e.g. 08:00"))?,
        m.trim().parse().map_err(|_| anyhow::anyhow!("'{at}' is not a time: use HH:MM, e.g. 08:00"))?,
    );
    if h > 23 || m > 59 {
        anyhow::bail!("'{at}' is not a time of day");
    }
    Ok(format!("{h:02}:{m:02}"))
}

pub async fn handle_push_command(cmd: PushCommands, tendril_home: &Path) -> anyhow::Result<()> {
    let mut cfg = read(tendril_home);
    match cmd {
        PushCommands::Set { url, token } => {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                anyhow::bail!("The push URL must start with http:// or https://");
            }
            cfg["url"] = json!(url.trim());
            match token {
                Some(t) if !t.trim().is_empty() => cfg["token"] = json!(t.trim()),
                _ => {
                    cfg.as_object_mut().map(|o| o.remove("token"));
                }
            }
            write(tendril_home, &cfg)?;
            println!("Pushes will go to {}. Run `tendril push test` to check it reaches your phone.", url.trim());
        }
        PushCommands::Briefing { at, utc_offset_minutes, off } => {
            if off {
                cfg.as_object_mut().map(|o| o.remove("briefing"));
                write(tendril_home, &cfg)?;
                println!("The morning briefing is off.");
            } else {
                let at = at.ok_or_else(|| anyhow::anyhow!("Give a time with --at HH:MM, or --off"))?;
                cfg["briefing"] = json!({ "time": validate_time(&at)?, "utcOffsetMinutes": utc_offset_minutes });
                write(tendril_home, &cfg)?;
                println!("Each active manager will send a briefing at {} (UTC{:+}).", validate_time(&at)?, utc_offset_minutes / 60);
                if cfg["url"].as_str().unwrap_or("").is_empty() {
                    println!("No push URL is set yet, so briefings will only appear in each manager's chat. Set one with `tendril push set <url>`.");
                }
            }
        }
        PushCommands::Test => {
            let url = cfg["url"].as_str().filter(|u| !u.is_empty()).map(str::to_string)
                .or_else(|| std::env::var("TENDRIL_PUSH_URL").ok())
                .ok_or_else(|| anyhow::anyhow!("No push URL is set. Use `tendril push set <url>`."))?;
            let mut req = reqwest::Client::new()
                .post(&url)
                .header("Title", "Tendril test")
                .body("If you can read this, pushes from your managers will reach you.");
            if let Some(token) = cfg["token"].as_str() {
                req = req.bearer_auth(token);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("The push service answered {}", resp.status());
            }
            println!("Sent. It should arrive on your phone in a moment.");
        }
        PushCommands::Off => {
            let _ = std::fs::remove_file(path(tendril_home));
            println!("Pushes and the briefing are off.");
        }
        PushCommands::Status => {
            match cfg["url"].as_str().filter(|u| !u.is_empty()) {
                Some(url) => println!("Pushes go to {url}{}", if cfg["token"].is_string() { " (with a token)" } else { "" }),
                None => println!("Pushes are off. Set a topic with `tendril push set <url>`."),
            }
            match cfg["briefing"]["time"].as_str() {
                Some(t) => println!("Morning briefing at {t} (UTC{:+}).", cfg["briefing"]["utcOffsetMinutes"].as_i64().unwrap_or(0) / 60),
                None => println!("No morning briefing."),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_time;

    #[test]
    fn times_are_validated_and_normalised() {
        assert_eq!(validate_time("8:05").unwrap(), "08:05");
        assert_eq!(validate_time("23:59").unwrap(), "23:59");
        assert!(validate_time("24:00").is_err());
        assert!(validate_time("08").is_err());
        assert!(validate_time("soon").is_err());
        assert!(validate_time("08:61").is_err());
    }
}
