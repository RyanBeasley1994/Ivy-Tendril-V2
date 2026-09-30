//! `tendril set-password` — turns on session protection (or changes its password) in one step.
//!
//! The same write Settings > Security & Tunneling makes through `PUT /api/auth/password`, for a
//! server with no desktop UI: a VPS the desktop app connects to remotely. `hash-password` produces the
//! same values but leaves copying them into `config.yaml` to the operator.
//!
//! The password is read from the terminal with echo off, or from the first line of stdin when stdin
//! is not a terminal (`echo "$PW" | tendril set-password`), so it never lands in shell history.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use tendril_core::auth::credentials::{is_password_active, set_password};
use tendril_core::config::get_config_path;

pub fn handle_set_password(tendril_home: &Path, current: Option<String>) -> Result<()> {
    let config_path = get_config_path(tendril_home);
    let interactive = std::io::stdin().is_terminal();

    let current = match current {
        Some(current) => Some(current),
        None if is_password_active(&config_path) && interactive => {
            Some(prompt_hidden("Current password: ")?)
        }
        None => None,
    };

    let new_password = if interactive {
        let first = prompt_hidden("New password: ")?;
        let again = prompt_hidden("Confirm new password: ")?;
        if first != again {
            bail!("The passwords do not match");
        }
        first
    } else {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .context("Failed to read the password from stdin")?;
        line.trim_end_matches(['\r', '\n']).to_string()
    };

    set_password(&config_path, current.as_deref(), &new_password)?;
    println!(
        "Password protection is on ({}). A running server picks it up without a restart.",
        config_path.display()
    );
    Ok(())
}

fn prompt_hidden(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let echo_off = set_echo(false);
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if echo_off {
        set_echo(true);
        eprintln!();
    }
    read.context("Failed to read the password")?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// Toggles terminal echo through `stty`, answering whether it took effect. Windows has no `stty`,
/// so there the password is typed visibly rather than not at all.
fn set_echo(on: bool) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("stty")
            .arg(if on { "echo" } else { "-echo" })
            .stdin(std::process::Stdio::inherit())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = on;
        false
    }
}
