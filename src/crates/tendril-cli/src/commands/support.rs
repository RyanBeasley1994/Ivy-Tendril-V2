//! `tendril support`: Propriotec Operations tickets (`tendril_core::support`), for a manager working a
//! ticket it was handed and for anyone at a terminal.
//!
//! Calls Operations directly with the connection the Support page saved in `<TendrilHome>`, so it runs
//! wherever the daemon does, with or without the daemon up.

use clap::Subcommand;
use std::path::Path;
use tendril_core::support::{self, ReplyOutcome};
use tendril_core::support_cases::{self, Category, Investigation, ReplyKind, ReplyRequest, Severity};
use tendril_core::support_data::{self, MongoRead};
use tendril_core::support_kube;

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

    #[command(about = "Reply to the partner. Checked against the triage rules, and held for the operator's approval while approval is on")]
    Reply {
        id: String,
        message: String,
        /// ack (a bug: we're on it), answer (a question), or resolved (a verified micro fix).
        #[arg(long, value_parser = ["ack", "answer", "resolved"])]
        kind: Option<String>,
        /// The fix's pull request, for --kind resolved.
        #[arg(long)]
        fix_pr: Option<String>,
        /// The test that reproduces the bug and now passes, for --kind resolved.
        #[arg(long)]
        test: Option<String>,
        /// Resolve the ticket once the reply is sent.
        #[arg(long)]
        close: bool,
        /// Same as `tendril support note`.
        #[arg(long)]
        internal: bool,
    },

    #[command(about = "An internal note on a ticket: only the team sees it")]
    Note { id: String, message: String },

    #[command(about = "Move a ticket: PENDING, OPEN, IN_PROGRESS, AWAITING_REPLY, RESOLVED or CLOSED")]
    Status { id: String, status: String },

    #[command(about = "Tell the partner (with --message, through the triage rules) and resolve the ticket")]
    Close {
        id: String,
        /// What the partner is told before it is resolved. Omit to resolve silently.
        #[arg(long)]
        message: Option<String>,
        #[arg(long, default_value = "resolved", value_parser = ["ack", "answer", "resolved"])]
        kind: String,
        #[arg(long)]
        fix_pr: Option<String>,
        #[arg(long)]
        test: Option<String>,
    },

    #[command(about = "Record what an investigation found: required before any reply")]
    Investigate {
        id: String,
        #[arg(long, value_parser = ["bug", "question", "request"])]
        category: String,
        #[arg(long, value_parser = ["critical", "high", "normal", "low"])]
        severity: String,
        /// 0-100: how sure you are.
        #[arg(long)]
        confidence: u8,
        #[arg(long)]
        findings: String,
        /// What you checked: file:line, a collection and record, an endpoint. Repeatable.
        #[arg(long)]
        evidence: Vec<String>,
        /// The ticket this one is a duplicate of.
        #[arg(long)]
        duplicate_of: Option<String>,
    },

    #[command(about = "Show a ticket's triage case: lookups, investigation and drafts")]
    Case {
        id: String,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },

    #[command(about = "List the data sources the support lookups can read")]
    Sources,

    #[command(about = "A read-only MongoDB lookup for a ticket, logged on it as evidence")]
    Db {
        /// The data source's name (`tendril support sources`).
        source: String,
        #[arg(long)]
        ticket: String,
        /// The database, when the connection string names none.
        #[arg(long)]
        db: Option<String>,
        #[arg(value_parser = ["find", "count", "aggregate", "collections"])]
        op: String,
        collection: Option<String>,
        #[arg(long, default_value = "")]
        filter: String,
        #[arg(long, default_value = "")]
        projection: String,
        #[arg(long, default_value = "")]
        sort: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
        #[arg(long, default_value = "")]
        pipeline: String,
    },

    #[command(about = "A read-only GET against an API data source for a ticket, logged on it as evidence")]
    Api {
        source: String,
        #[arg(long)]
        ticket: String,
        /// A path under the source's base URL, like /api/accounts/123.
        path: String,
    },

    #[command(about = "Read the cluster with the server's kubeconfig: get, describe, logs, top, events (no changes, no secret values)")]
    Kube {
        #[arg(long)]
        ticket: String,
        /// The kubectl arguments, e.g. `get pods -n prod` or `logs deploy/server -n prod --since=1h`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },

    #[command(about = "A Kubernetes Secret's key names and sizes (never its values)")]
    KubeSecret {
        /// `namespace/name`.
        secret: String,
        #[arg(long)]
        ticket: String,
    },

    #[command(about = "A read-only MongoDB lookup on the database whose connection string is in a Secret (port-forwarded when in-cluster)")]
    KubeDb {
        /// `namespace/secret:key` holding the connection string.
        secret: String,
        #[arg(long)]
        ticket: String,
        #[arg(long)]
        db: Option<String>,
        #[arg(value_parser = ["find", "count", "aggregate", "collections"])]
        op: String,
        collection: Option<String>,
        #[arg(long, default_value = "")]
        filter: String,
        #[arg(long, default_value = "")]
        projection: String,
        #[arg(long, default_value = "")]
        sort: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
        #[arg(long, default_value = "")]
        pipeline: String,
    },

    #[command(about = "A GET to an in-cluster service through a port-forward")]
    KubeApi {
        /// `namespace/svc-name:port`, or `namespace/pod/name:port`.
        target: String,
        path: String,
        #[arg(long)]
        ticket: String,
        /// `secret-name:key` in the same namespace, sent as a bearer token.
        #[arg(long)]
        bearer: Option<String>,
    },

    #[command(about = "Search how past tickets were settled")]
    History {
        /// Words to match. Omit for the latest.
        query: Option<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },

    #[command(about = "Write how a ticket was fixed or settled into the support history")]
    Learn { id: String, resolution: String },
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
        SupportCommands::Reply { id, message, kind, fix_pr, test, close, internal } => {
            if internal {
                support::agent_note(home, &id, &message).await?;
                println!("Internal note posted on {id}.");
                return Ok(());
            }
            let kind = kind.ok_or_else(|| anyhow::anyhow!("Say what kind of reply this is: --kind ack, answer or resolved"))?;
            let req = reply_request(&kind, message, fix_pr, test, close);
            print_outcome(&id, support::agent_reply(home, &id, req).await?);
        }
        SupportCommands::Note { id, message } => {
            support::agent_note(home, &id, &message).await?;
            println!("Internal note posted on {id}.");
        }
        SupportCommands::Status { id, status } => {
            support::set_status(home, &id, &status).await?;
            println!("Ticket {id} is now {}.", status.to_ascii_uppercase());
        }
        SupportCommands::Close { id, message, kind, fix_pr, test } => match message {
            Some(m) if !m.trim().is_empty() => {
                print_outcome(&id, support::agent_reply(home, &id, reply_request(&kind, m, fix_pr, test, true)).await?);
            }
            _ => {
                support::resolve(home, &id).await?;
                println!("Ticket {id} resolved.");
            }
        },
        SupportCommands::Investigate { id, category, severity, confidence, findings, evidence, duplicate_of } => {
            let category = match category.as_str() {
                "bug" => Category::Bug,
                "question" => Category::Question,
                _ => Category::Request,
            };
            let severity = match severity.as_str() {
                "critical" => Severity::Critical,
                "high" => Severity::High,
                "low" => Severity::Low,
                _ => Severity::Normal,
            };
            let investigation = Investigation {
                at: chrono::Utc::now(),
                category,
                severity,
                confidence,
                findings,
                evidence: evidence.into_iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect(),
                duplicate_of: duplicate_of.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
            };
            let ticket_type = support::type_for(&investigation);
            let case = support_cases::record_investigation(home, &id, investigation)?;
            if let Err(e) = support::set_type(home, &id, ticket_type).await {
                eprintln!("Could not set the ticket's type to {ticket_type} in Operations: {e}");
            }
            println!(
                "Investigation recorded on {id} ({} lookups logged). {}",
                case.lookups.len(),
                match category {
                    Category::Request => "It is a request: send no reply.",
                    _ if confidence < support_cases::ANSWER_CONFIDENCE => "Below 90%: write an internal note, not a reply.",
                    _ => "You may reply within the rules.",
                }
            );
        }
        SupportCommands::Case { id, json } => {
            let case = support_cases::load_case(home, &id);
            if json {
                println!("{}", serde_json::to_string_pretty(&case)?);
                return Ok(());
            }
            match &case.investigation {
                Some(i) => println!(
                    "{:?} · {:?} · {}% sure\n{}\nEvidence: {}",
                    i.category,
                    i.severity,
                    i.confidence,
                    i.findings,
                    if i.evidence.is_empty() { "-".to_string() } else { i.evidence.join("; ") }
                ),
                None => println!("Not investigated yet."),
            }
            println!("\nLookups ({}):", case.lookups.len());
            for l in &case.lookups {
                println!("- {} {}: {} → {}", l.at.format("%H:%M"), l.source, l.request, l.result);
            }
            println!("\nDrafts:");
            for d in &case.drafts {
                println!("- [{:?}] {:?}: {}", d.state, d.kind, d.content);
            }
        }
        SupportCommands::Sources => {
            let settings = support_cases::load_settings(home);
            if settings.sources.is_empty() {
                println!("No data sources. Add them on the Support page (Support settings).");
            }
            for s in support_cases::view(&settings).sources {
                println!("{}  [{:?}] {}{}", s.name, s.kind, s.url, if s.notes.is_empty() { String::new() } else { format!("  — {}", s.notes) });
            }
        }
        SupportCommands::Db { source, ticket, db, op, collection, filter, projection, sort, limit, pipeline } => {
            let need = |c: Option<String>| c.ok_or_else(|| anyhow::anyhow!("Name the collection"));
            let read = match op.as_str() {
                "collections" => MongoRead::Collections,
                "count" => MongoRead::Count { collection: need(collection)?, filter },
                "aggregate" => MongoRead::Aggregate { collection: need(collection)?, pipeline },
                _ => MongoRead::Find { collection: need(collection)?, filter, projection, sort, limit },
            };
            println!("{}", support_data::mongo_read(home, &ticket, &source, db.as_deref(), read).await?);
        }
        SupportCommands::Api { source, ticket, path } => {
            println!("{}", support_data::api_get(home, &ticket, &source, &path).await?);
        }
        SupportCommands::Kube { ticket, args } => {
            println!("{}", support_kube::read(home, &ticket, args).await?);
        }
        SupportCommands::KubeSecret { secret, ticket } => {
            let (ns, name) = secret.split_once('/').ok_or_else(|| anyhow::anyhow!("Name it as namespace/name"))?;
            println!("{}", support_kube::secret_keys(home, &ticket, ns, name).await?);
        }
        SupportCommands::KubeDb { secret, ticket, db, op, collection, filter, projection, sort, limit, pipeline } => {
            let (ns, rest) = secret.split_once('/').ok_or_else(|| anyhow::anyhow!("Name it as namespace/secret:key"))?;
            let (name, key) = rest.split_once(':').ok_or_else(|| anyhow::anyhow!("Name the key too: namespace/secret:key"))?;
            let need = |c: Option<String>| c.ok_or_else(|| anyhow::anyhow!("Name the collection"));
            let read = match op.as_str() {
                "collections" => MongoRead::Collections,
                "count" => MongoRead::Count { collection: need(collection)?, filter },
                "aggregate" => MongoRead::Aggregate { collection: need(collection)?, pipeline },
                _ => MongoRead::Find { collection: need(collection)?, filter, projection, sort, limit },
            };
            println!("{}", support_kube::database(home, &ticket, (ns, name, key), db.as_deref(), read).await?);
        }
        SupportCommands::KubeApi { target, path, ticket, bearer } => {
            let (ns, rest) = target.split_once('/').ok_or_else(|| anyhow::anyhow!("Name it as namespace/service:port"))?;
            let (svc, port) = rest.rsplit_once(':').ok_or_else(|| anyhow::anyhow!("Name the port: namespace/service:port"))?;
            let port: u16 = port.parse().map_err(|_| anyhow::anyhow!("'{port}' is not a port"))?;
            let bearer = match bearer.as_deref() {
                Some(b) => Some(b.split_once(':').ok_or_else(|| anyhow::anyhow!("--bearer is secret-name:key"))?),
                None => None,
            };
            println!("{}", support_kube::api(home, &ticket, ns, svc, port, &path, bearer).await?);
        }
        SupportCommands::History { query, limit } => {
            let entries = match query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
                Some(q) => support_cases::similar(home, q, None, limit),
                None => {
                    let all = support_cases::load_history(home);
                    all.into_iter().rev().take(limit).collect()
                }
            };
            if entries.is_empty() {
                println!("Nothing like that in the support history yet.");
            }
            print!("{}", support_cases::render_history(&entries));
        }
        SupportCommands::Learn { id, resolution } => {
            let case = support_cases::load_case(home, &id);
            let partner = support::get_ticket(home, &id)
                .await
                .ok()
                .and_then(|t| t["partner"]["name"].as_str().map(str::to_string));
            let mut entry = support_cases::entry_from_case(home, &case, partner, Some(&resolution));
            if entry.subject.is_empty() {
                entry.subject = support::get_ticket(home, &id)
                    .await
                    .ok()
                    .and_then(|t| t["subject"].as_str().map(str::to_string))
                    .unwrap_or_default();
            }
            support_cases::remember(home, &entry)?;
            println!("Saved to the support history: {}", entry.subject);
        }
    }
    Ok(())
}

fn reply_request(kind: &str, content: String, fix_pr: Option<String>, test: Option<String>, close: bool) -> ReplyRequest {
    let kind = match kind {
        "ack" => ReplyKind::Ack,
        "answer" => ReplyKind::Answer,
        _ => ReplyKind::Resolved,
    };
    ReplyRequest { kind, content, close, fix_pr, test }
}

fn print_outcome(id: &str, outcome: ReplyOutcome) {
    match outcome {
        ReplyOutcome::Queued { .. } => {
            println!("Held for the operator's approval on ticket {id}: they approve or edit it before the partner sees it.")
        }
        ReplyOutcome::Sent => println!("Reply sent on {id}."),
    }
}
