//! `tendril support`: Propriotec Operations tickets (`tendril_core::support`), for a manager working a
//! ticket it was handed and for anyone at a terminal.
//!
//! Calls Operations directly with the connection the Support page saved in `<TendrilHome>`, so it runs
//! wherever the daemon does, with or without the daemon up.

use clap::Subcommand;
use std::path::Path;
use tendril_core::support;

#[derive(Subcommand)]
pub enum SupportCommands {
    #[command(about = "List open tickets (the Operations board)")]
    List {
        /// Include resolved and closed tickets.
        #[arg(long)]
        all: bool,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "Show a ticket and its whole conversation, internal notes included")]
    Get {
        id: String,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "Reply on a ticket. With --internal only the team sees it")]
    Reply {
        id: String,
        message: String,
        #[arg(long)]
        internal: bool,
    },

    #[command(about = "Move a ticket: PENDING, OPEN, IN_PROGRESS, AWAITING_REPLY, RESOLVED or CLOSED")]
    Status { id: String, status: String },

    #[command(about = "Tell the partner (with --message) and resolve the ticket")]
    Close {
        id: String,
        /// What the partner is told before it is resolved. Omit to resolve silently.
        #[arg(long)]
        message: Option<String>,
    },
}

pub async fn handle_support_command(cmd: SupportCommands, home: &Path) -> anyhow::Result<()> {
    match cmd {
        SupportCommands::List { all, json } => {
            let v = support::list_tickets(home, all).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v["tickets"])?);
                return Ok(());
            }
            let tickets = v["tickets"].as_array().cloned().unwrap_or_default();
            if tickets.is_empty() {
                println!("No tickets.");
            }
            for t in tickets {
                println!(
                    "{}  [{}/{}] {} — {}{}",
                    t["id"].as_str().unwrap_or("?"),
                    t["status"].as_str().unwrap_or("?"),
                    t["priority"].as_str().unwrap_or("?"),
                    t["partner"]["name"].as_str().unwrap_or("?"),
                    t["subject"].as_str().unwrap_or(""),
                    t["handoff"]["project"].as_str().map(|p| format!("  (with {p})")).unwrap_or_default(),
                );
            }
        }
        SupportCommands::Get { id, json } => {
            let t = support::get_ticket(home, &id).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&t)?);
                return Ok(());
            }
            println!("# {}", t["subject"].as_str().unwrap_or(""));
            println!(
                "{} · {} · {} · partner {}",
                t["status"].as_str().unwrap_or("?"),
                t["priority"].as_str().unwrap_or("?"),
                t["platform"].as_str().unwrap_or("?"),
                t["partner"]["name"].as_str().unwrap_or("?"),
            );
            for m in t["messages"].as_array().into_iter().flatten() {
                let internal = if m["isInternal"].as_bool().unwrap_or(false) { " [internal]" } else { "" };
                println!(
                    "\n{} — {}{}\n{}",
                    m["createdAt"].as_str().unwrap_or_default(),
                    m["authorName"].as_str().unwrap_or("?"),
                    internal,
                    m["content"].as_str().unwrap_or_default().trim()
                );
            }
        }
        SupportCommands::Reply { id, message, internal } => {
            support::reply(home, &id, &message, internal).await?;
            println!("{} posted on {id}.", if internal { "Internal note" } else { "Reply" });
        }
        SupportCommands::Status { id, status } => {
            support::set_status(home, &id, &status).await?;
            println!("Ticket {id} is now {}.", status.to_ascii_uppercase());
        }
        SupportCommands::Close { id, message } => {
            support::close(home, &id, message.as_deref()).await?;
            println!("Ticket {id} resolved{}.", if message.is_some() { " and the partner told" } else { "" });
        }
    }
    Ok(())
}
