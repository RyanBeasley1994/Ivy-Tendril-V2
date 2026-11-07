//! The support triage loop: new tickets go to the support manager by themselves.
//!
//! Every minute, while auto triage is on (`tendril_core::support_cases::SupportSettings`), the board
//! is read and each ticket that arrived since triage was turned on, and that nobody has handed over,
//! is given to the chosen project's manager with the triage brief (`support::triage_prompt`). Its
//! conversation is the same ticket chat a manual hand-off opens, so the operator can step in there.
//!
//! The loop also alerts the operator (Telegram, when paired) to a ticket that came in critical or was
//! found to be, and passes a partner's follow-up message on to the ticket's conversation.

use crate::routes::support::{find_project, send_to_session, start_ticket_conversation};
use crate::state::AppState;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tendril_core::support;
use tendril_core::support_cases::{self, Severity};

const EVERY: Duration = Duration::from_secs(60);
const FIRST_RUN_DELAY: Duration = Duration::from_secs(20);

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_RUN_DELAY).await;
        loop {
            if let Err(e) = pass(&state).await {
                tracing::debug!("support triage pass: {e}");
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// A short fingerprint of a message, to tell whether it was already passed on.
fn fingerprint(m: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    m["authorName"].as_str().unwrap_or_default().hash(&mut h);
    m["content"].as_str().unwrap_or_default().hash(&mut h);
    format!("{:016x}", h.finish())
}

async fn pass(state: &Arc<AppState>) -> Result<(), String> {
    let home = &state.tendril_home;
    if !support::status(home).configured {
        return Ok(());
    }
    let settings = support_cases::load_settings(home);
    let tickets = support::list_tickets(home, false).await.map_err(|e| e.to_string())?;
    let tickets = tickets["tickets"].as_array().cloned().unwrap_or_default();

    let project = settings
        .manager_project
        .as_deref()
        .filter(|_| settings.auto_triage)
        .and_then(|p| find_project(state, p));
    let since = settings.triage_since.unwrap_or_else(Utc::now);

    for t in &tickets {
        let Some(id) = t["id"].as_str() else { continue };
        let case = support_cases::load_case(home, id);
        let created = t["createdAt"].as_str().and_then(|c| c.parse::<DateTime<Utc>>().ok());

        // 1. New tickets go to the support manager.
        if let Some(project) = &project {
            let new = case.triaged_at.is_none() && t["handoff"].is_null() && created.is_some_and(|c| c >= since);
            if new {
                triage(state, project, id).await;
                continue;
            }
        }

        // 2. Critical tickets reach the operator at once.
        let critical_in = t["priority"].as_str() == Some("CRITICAL");
        let critical_found = case.investigation.as_ref().is_some_and(|i| i.severity == Severity::Critical);
        if (critical_in || critical_found) && !case.alerted && case.triaged_at.is_some() {
            alert(state, &settings, t, &case).await;
        }

        // 3. A reply held for approval is offered on Telegram, with buttons, once.
        for draft in case.drafts.iter().filter(|d| d.state == support_cases::DraftState::Pending) {
            if !case.offered_drafts.contains(&draft.id) && crate::telegram::offer_draft(state, id, &case.subject, draft).await {
                let draft_id = draft.id.clone();
                let _ = support_cases::update_case(home, id, |c| c.offered_drafts.push(draft_id));
            }
        }

        // 4. Waiting on us too long: one nudge per waiting stretch, before the response time runs out.
        if let Some(waiting) = t["waitingSince"].as_str().filter(|_| t["needsReply"].as_bool() == Some(true)) {
            let long = waiting.parse::<DateTime<Utc>>().is_ok_and(|w| Utc::now() - w >= chrono::Duration::hours(NUDGE_AFTER_HOURS));
            if long && case.nudged_for.as_deref() != Some(waiting) {
                nudge(state, &settings, t).await;
                let waiting = waiting.to_string();
                let _ = support_cases::update_case(home, id, |c| c.nudged_for = Some(waiting));
            }
        }

        // 5. The partner wrote back after being told it was fixed: the fix did not hold, and the ticket
        //    goes straight back to the manager as a regression.
        let partner_wrote = t["lastMessage"]["authorType"].as_str() == Some("PARTNER");
        let wrote_at = t["lastMessage"]["createdAt"].as_str().and_then(|c| c.parse::<DateTime<Utc>>().ok());
        if let (Some(resolved), Some(at), Some(project)) = (case.resolved_at(), wrote_at, &project) {
            if partner_wrote && at > resolved && !case.reopened {
                reopen(state, project, id, &t["lastMessage"]).await;
                continue;
            }
        }

        // 6. A partner's follow-up is passed on to the ticket's conversation.
        let open = t["handoff"]["doneAt"].is_null() && !t["handoff"].is_null();
        if case.triaged_at.is_some() && partner_wrote && open {
            let print = fingerprint(&t["lastMessage"]);
            if case.last_forwarded.as_deref() != Some(print.as_str()) {
                if let Some(session) = t["handoff"]["sessionId"].as_str() {
                    let text = t["lastMessage"]["content"].as_str().unwrap_or_default().trim();
                    let prompt = format!(
                        "The partner wrote again on this ticket:\n\n> {}\n\nInvestigate anything new before replying, and keep to the triage rules.",
                        text.replace('\n', "\n> ")
                    );
                    if send_to_session(state, session, prompt).await.is_ok() {
                        let _ = support_cases::update_case(home, id, |c| c.last_forwarded = Some(print));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn triage(state: &Arc<AppState>, project: &str, id: &str) {
    let home = &state.tendril_home;
    let ticket = match support::get_ticket(home, id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("support triage could not read ticket {id}: {e}");
            return;
        }
    };
    let subject = ticket["subject"].as_str().unwrap_or_default().to_string();
    let last = ticket["messages"].as_array().and_then(|m| m.last()).map(fingerprint);
    // Marked first, so a failure below is not retried every minute into a pile of conversations.
    let _ = support_cases::update_case(home, id, |c| {
        c.triaged_at = Some(Utc::now());
        c.subject = subject.clone();
        c.last_forwarded = last;
    });
    let prompt = support::triage_prompt(home, &ticket, project);
    match start_ticket_conversation(state, project, id, &ticket, prompt, None).await {
        Ok(_) => tracing::info!("Support ticket {id} ({subject}) handed to {project}'s manager for triage"),
        Err(_) => tracing::warn!("Support ticket {id} could not be handed to {project}'s manager"),
    }
    if ticket["priority"].as_str() == Some("CRITICAL") {
        let case = support_cases::load_case(home, id);
        alert(state, &support_cases::load_settings(home), &ticket, &case).await;
    }
}

/// Hours a ticket may wait on a reply from us before the operator is nudged, once.
const NUDGE_AFTER_HOURS: i64 = 4;

async fn nudge(state: &Arc<AppState>, settings: &support_cases::SupportSettings, ticket: &Value) {
    let project = settings.manager_project.clone().unwrap_or_else(|| "Support".into());
    let hours = ticket["waitingSince"]
        .as_str()
        .and_then(|w| w.parse::<DateTime<Utc>>().ok())
        .map(|w| (Utc::now() - w).num_hours())
        .unwrap_or(NUDGE_AFTER_HOURS);
    let body = format!(
        "{} ({}) has waited {hours}h for a reply.",
        ticket["subject"].as_str().unwrap_or("A ticket"),
        ticket["partner"]["name"].as_str().unwrap_or("unknown partner"),
    );
    crate::telegram::notify(state, &project, "Support ticket waiting", &body).await;
}

/// Back to the manager, as a regression: the partner says it is still broken after "resolved".
async fn reopen(state: &Arc<AppState>, project: &str, id: &str, message: &Value) {
    let home = &state.tendril_home;
    let Ok(ticket) = support::get_ticket(home, id).await else { return };
    let text = message["content"].as_str().unwrap_or_default().trim();
    let note = format!(
        "REGRESSION: the partner wrote back after being told this was resolved:\n\n> {}\n\nTreat it as the fix not holding. Investigate again (record a fresh investigation), check whether the fix actually shipped and covers what they describe, and keep to the triage rules. No new \"resolved\" until you are certain.",
        text.replace('\n', "\n> ")
    );
    let print = fingerprint(message);
    let _ = support_cases::update_case(home, id, |c| {
        c.reopened = true;
        c.handed_over = false;
        c.last_forwarded = Some(print);
    });
    let first = format!("{note}\n\n{}", support::triage_prompt(home, &ticket, project));
    match start_ticket_conversation(state, project, id, &ticket, first, Some(note)).await {
        Ok(_) => tracing::info!("Support ticket {id} reopened as a regression for {project}'s manager"),
        Err(_) => tracing::warn!("Support ticket {id} could not be reopened for {project}'s manager"),
    }
    let _ = support::set_status(home, id, "IN_PROGRESS").await;
}

async fn alert(state: &Arc<AppState>, settings: &support_cases::SupportSettings, ticket: &Value, case: &support_cases::Case) {
    let id = ticket["id"].as_str().unwrap_or_default();
    let project = settings.manager_project.clone().unwrap_or_else(|| "Support".into());
    let why = match &case.investigation {
        Some(i) if i.severity == Severity::Critical => format!("Found critical: {}", i.findings),
        _ => "Came in marked critical.".to_string(),
    };
    let body = format!(
        "{} ({})\n{}",
        ticket["subject"].as_str().unwrap_or("A ticket"),
        ticket["partner"]["name"].as_str().unwrap_or("unknown partner"),
        why
    );
    crate::telegram::notify(state, &project, "Critical support ticket", &body).await;
    let _ = support_cases::update_case(&state.tendril_home, id, |c| c.alerted = true);
}
