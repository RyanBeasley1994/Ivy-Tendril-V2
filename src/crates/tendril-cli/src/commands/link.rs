//! `tendril link`: what a project has hosted (`tendril_core::project_links`), for agents to record a
//! URL and its test login so it shows on the project page instead of scrolling away in a chat.
//!
//! File edits under `<TendrilHome>/Projects/<project>`, so these work with or without a running
//! daemon, the same way `tendril memory` does.

use clap::Subcommand;
use std::path::Path;
use tendril_core::project_links::{self, LinkInput};

#[derive(Subcommand)]
pub enum LinkCommands {
    #[command(about = "List what a project has hosted")]
    List {
        #[arg(long)]
        project: String,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "Record a hosted URL (with an optional test login). The same URL again updates it")]
    Set {
        #[arg(long)]
        project: String,
        #[arg(long)]
        url: String,
        /// What it is: "Dev server", "Preview deploy", "Staging admin".
        #[arg(long, default_value = "")]
        label: String,
        /// A test account's user name or email.
        #[arg(long, default_value = "")]
        username: String,
        /// A test account's password. Never a real secret: this is stored in plain text.
        #[arg(long, default_value = "")]
        password: String,
        #[arg(long, default_value = "")]
        notes: String,
        /// Update this link instead of matching by URL.
        #[arg(long)]
        slug: Option<String>,
        /// Who recorded it: plan:ID, job:ID, mission:ID, chat:ID or user.
        #[arg(long)]
        source: Option<String>,
    },

    #[command(about = "Remove a link that is no longer up")]
    Remove {
        #[arg(long)]
        project: String,
        slug: String,
    },
}

pub async fn handle_link_command(cmd: LinkCommands, tendril_home: &Path) -> anyhow::Result<()> {
    match cmd {
        LinkCommands::List { project, json } => {
            let links = project_links::list(tendril_home, &project);
            if json {
                println!("{}", serde_json::to_string_pretty(&links)?);
            } else if links.is_empty() {
                println!("Nothing hosted for project {} yet.", project);
            } else {
                for l in links {
                    let login = if l.username.is_empty() && l.password.is_empty() {
                        String::new()
                    } else {
                        format!(" ({} / {})", l.username, l.password)
                    };
                    println!("{} {} — {}{}", l.slug, l.label, l.url, login);
                }
            }
        }
        LinkCommands::Set { project, url, label, username, password, notes, slug, source } => {
            let saved = project_links::set(
                tendril_home,
                &project,
                LinkInput { slug, label, url, username, password, notes, source },
            )?;
            println!("Link saved: {} — {}", saved.slug, saved.url);
        }
        LinkCommands::Remove { project, slug } => {
            if project_links::remove(tendril_home, &project, &slug)? {
                println!("Link removed: {}", slug);
            } else {
                println!("No link {} in project {}.", slug, project);
            }
        }
    }
    Ok(())
}
