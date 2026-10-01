//! `tendril memory`: a project's long-term memory (`tendril_core::project_memory`), for agents to
//! save what they learn about a project and for anyone to read, correct or prune it.
//!
//! File edits under `<TendrilHome>/Projects/<project>/Memory`, so these work with or without a
//! running daemon, the same way `tendril promptware write-memory` does.

use clap::Subcommand;
use std::io::{IsTerminal, Read};
use std::path::Path;
use tendril_core::project_memory::{self, MemoryInput};

#[derive(Subcommand)]
pub enum MemoryCommands {
    #[command(about = "List a project's memories")]
    List {
        #[arg(long)]
        project: String,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "Show one memory in full")]
    Get {
        #[arg(long)]
        project: String,
        /// The memory's id (its slug), as `list` prints it.
        slug: String,
    },

    #[command(about = "Save a memory, or update one with --slug. The body is read from stdin")]
    Write {
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        /// architecture, convention, decision, gotcha, preference or note.
        #[arg(long = "type", default_value = "note")]
        kind: String,
        /// One line: what this memory is about, used to decide relevance.
        #[arg(long, default_value = "")]
        description: String,
        /// A repo-relative file or folder this memory is about. Repeatable.
        #[arg(long = "path")]
        paths: Vec<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Update this memory instead of creating one.
        #[arg(long)]
        slug: Option<String>,
        /// Who learned it: plan:ID, job:ID, mission:ID, chat:ID or user.
        #[arg(long)]
        source: Option<String>,
        /// The body inline, instead of stdin.
        #[arg(long)]
        body: Option<String>,
    },

    #[command(about = "Delete a memory that is wrong or no longer true")]
    Delete {
        #[arg(long)]
        project: String,
        slug: String,
    },

    #[command(about = "Show the memories most relevant to a description of some work")]
    Recall {
        #[arg(long)]
        project: String,
        query: String,
        #[arg(long = "path")]
        paths: Vec<String>,
        #[arg(long, default_value_t = 6)]
        limit: usize,
    },
}

pub async fn handle_memory_command(cmd: MemoryCommands, tendril_home: &Path) -> anyhow::Result<()> {
    match cmd {
        MemoryCommands::List { project, json } => {
            let entries = project_memory::list(tendril_home, &project);
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("No memories for project {} yet.", project);
            } else {
                for e in entries {
                    println!(
                        "{} [{}] {}{}",
                        e.slug,
                        e.kind.as_str(),
                        e.title,
                        if e.description.is_empty() { String::new() } else { format!(" — {}", e.description) }
                    );
                }
            }
        }
        MemoryCommands::Get { project, slug } => {
            let e = project_memory::get(tendril_home, &project, &slug)?;
            println!("# {} ({})", e.title, e.kind.as_str());
            if !e.description.is_empty() {
                println!("{}", e.description);
            }
            if !e.paths.is_empty() {
                println!("Paths: {}", e.paths.join(", "));
            }
            if !e.tags.is_empty() {
                println!("Tags: {}", e.tags.join(", "));
            }
            println!("Source: {} · updated {}", e.source, e.updated.format("%Y-%m-%d %H:%M"));
            println!("\n{}", e.body);
        }
        MemoryCommands::Write { project, title, kind, description, paths, tags, slug, source, body } => {
            let body = match body {
                Some(b) => b,
                None if !std::io::stdin().is_terminal() => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
                None => String::new(),
            };
            let saved = project_memory::write(
                tendril_home,
                &project,
                MemoryInput {
                    slug,
                    title,
                    kind: Some(kind),
                    description,
                    paths,
                    tags,
                    source,
                    body,
                },
            )?;
            println!("Memory saved: {}", saved.slug);
        }
        MemoryCommands::Delete { project, slug } => {
            if project_memory::delete(tendril_home, &project, &slug)? {
                println!("Memory deleted: {}", slug);
            } else {
                println!("No memory {} in project {}.", slug, project);
            }
        }
        MemoryCommands::Recall { project, query, paths, limit } => {
            let entries = project_memory::list(tendril_home, &project);
            for (score, e) in project_memory::recall(&entries, &query, &paths, limit) {
                println!("{:.2}  {} [{}] {}", score, e.slug, e.kind.as_str(), e.title);
            }
        }
    }
    Ok(())
}
