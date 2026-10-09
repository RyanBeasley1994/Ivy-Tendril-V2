//! The prompt one chat turn hands the agent, and the spawned-job block that opens it.

use crate::chat::manager_brief::is_manager_session;
use crate::chat::models::ChatMessage;

/// The prompt one chat turn hands the agent: the conversation so far, the chat session's id, and the
/// current request. Port of `ChatExecutionService.SendMessageAsync`'s `agentPromptBuilder`.
///
/// `history` is the session's messages *before* this turn's own user message and assistant stub were
/// appended, which is the same slice V1 replays (`history.Take(history.Count - 1)`).
///
/// Replaying the conversation is what carries it forward: a chat turn is a fresh agent session, so
/// nothing else remembers the previous turns. Empty messages are skipped, because an assistant stub
/// that a turn never filled in has nothing to contribute.
pub fn build_chat_agent_prompt(
    history: &[ChatMessage],
    prompt: &str,
    session_id: &str,
    role: &str,
    spawned_jobs: &[ChatSpawnedJob],
) -> String {
    build_chat_agent_prompt_with_state(history, prompt, session_id, role, spawned_jobs, None)
}

/// How many of a manager's latest messages are replayed. Its session lives as long as its project, and
/// replaying all of it made every wake cost more than the one before; what is older than this is either
/// in the Goals memory, where its briefing tells it to put anything that must last, or no longer matters.
pub const MANAGER_HISTORY_MESSAGES: usize = 24;
/// How much of an event the daemon sent earlier is replayed. It was acted on in the turn it arrived;
/// afterwards the first lines are enough to say what it was.
const MANAGER_PAST_EVENT_CHARS: usize = 400;

/// The history a manager is shown: its briefing, then only the latest messages, with old events clipped.
fn manager_history(prior: Vec<&ChatMessage>) -> Vec<(String, String)> {
    let label = |m: &ChatMessage| match m.role.to_ascii_lowercase().as_str() {
        "user" => "User",
        "system" => "System Event",
        _ => "Assistant",
    };
    let mut out = Vec::new();
    let (briefing, rest): (Vec<&ChatMessage>, Vec<&ChatMessage>) =
        prior.into_iter().partition(|m| m.role == "system" && crate::chat::manager_brief::is_briefing(&m.content));
    if let Some(b) = briefing.first() {
        out.push(("Your briefing".to_string(), b.content.clone()));
    }
    let skipped = rest.len().saturating_sub(MANAGER_HISTORY_MESSAGES);
    if skipped > 0 {
        out.push((
            "Earlier".to_string(),
            format!("({skipped} earlier messages are not shown. What still matters from them is in the Goals memory.)"),
        ));
    }
    for m in &rest[skipped..] {
        let content = if m.role.eq_ignore_ascii_case("system") && m.content.chars().count() > MANAGER_PAST_EVENT_CHARS {
            format!("{}… (clipped; already handled)", m.content.chars().take(MANAGER_PAST_EVENT_CHARS).collect::<String>())
        } else {
            m.content.clone()
        };
        out.push((label(m).to_string(), content));
    }
    out
}

/// [`build_chat_agent_prompt`], with a section on where things stand placed just before the request.
/// `state` is a manager's project snapshot ([`crate::chat::manager_snapshot`]); other chats pass `None`.
pub fn build_chat_agent_prompt_with_state(
    history: &[ChatMessage],
    prompt: &str,
    session_id: &str,
    role: &str,
    spawned_jobs: &[ChatSpawnedJob],
    state: Option<&str>,
) -> String {
    let mut out = String::new();
    // A project's manager is briefed to act and report in a line, never to advise and ask. The framing
    // every other chat gets ("guide the user through the next steps", "advise the user ... suggested next
    // steps") tells it the opposite on every single turn, so it gets its own.
    let manager = is_manager_session(session_id);

    // V1 puts this first, before the history: what the session has already set running is context for
    // everything below it, and for a turn that *is* a job event it is the subject. A manager's goes
    // after its history instead, next to the rest of what is current.
    if !manager && !spawned_jobs.is_empty() {
        out.push_str("# Jobs Spawned in this Chat Session\n");
        out.push_str("The following jobs were spawned in this chat session:\n\n");
        for job in spawned_jobs {
            out.push_str(&job.prompt_line());
        }
        out.push('\n');
        out.push_str(match JobRollup::of(spawned_jobs) {
            JobRollup::AllDone => "All spawned jobs have completed. Proactively guide the user through the next steps (e.g. ask if they want you to review the plan or implementation, inspect results, or proceed to creating PRs).\n",
            JobRollup::AnyFailed => "Some spawned jobs failed or encountered issues. Guide the user through the failures and offer to diagnose, retry, or adjust the plan.\n",
            JobRollup::StillRunning => "Some spawned jobs are still running or pending. Inform the user of their progress as appropriate.\n",
        });
        out.push_str("---\n\n");
    }

    let prior: Vec<&ChatMessage> = history
        .iter()
        .filter(|m| !m.content.trim().is_empty())
        .collect();
    if manager {
        if !prior.is_empty() {
            out.push_str("# Previous Conversation Discussion History\n\n");
            for (label, content) in manager_history(prior) {
                out.push_str(&format!("### {label}\n{content}\n\n"));
            }
            out.push_str("---\n\n");
        }
        out.push_str(state.unwrap_or_default());
        out.push_str(&manager_jobs_section(spawned_jobs));
    } else if !prior.is_empty() {
        out.push_str("# Previous Conversation Discussion History\n");
        out.push_str(
            "The following is the previous conversation history in this chat session:\n\n",
        );
        for msg in prior {
            let label = match msg.role.to_ascii_lowercase().as_str() {
                "user" => "User",
                "system" => "System Event",
                _ => "Assistant",
            };
            out.push_str(&format!("### {}\n{}\n\n", label, msg.content));
        }
        out.push_str("---\n\n");
    }

    out.push_str("# Current Chat Session\n");
    out.push_str(&format!("Chat Session ID: {}\n", session_id));
    // The whole command, in one code span, rather than the subcommand and the flag in two. That is what
    // the agent copies, and it is also what `promptware_contract_test` can hand to clap — the split
    // form named a flag the CLI did not have for as long as it took someone to notice the chat's jobs
    // menu was always empty, because neither half was a command anything could check.
    out.push_str(&format!(
        "When starting jobs, always pass `--chat-session {session_id}` so the job is tracked in \
         this chat session, e.g. `tendril job start ExecutePlan 00042 --chat-session {session_id}`.\n"
    ));
    out.push_str("---\n\n");

    if is_event_role(role) {
        // The framing is the whole point of the system role: without it an injected event reads as
        // something the user typed, and the agent answers it instead of reacting to it.
        out.push_str("# Current Event Notification\n");
        out.push_str(prompt);
        out.push_str("\n\n");
        out.push_str(if manager {
            MANAGER_EVENT_INSTRUCTION
        } else {
            "Evaluate this completed job event. Proactively inspect the job outcomes/artifacts if needed, determine whether any action is needed, and advise the user with a concise summary and suggested next steps.\n"
        });
    } else {
        out.push_str("# Current User Request\n");
        out.push_str(prompt);
        out.push('\n');
    }

    out
}

/// What a manager is told after an event the daemon woke it with.
const MANAGER_EVENT_INSTRUCTION: &str = "This came from the daemon, not from the operator, who may not be watching. Handle it the way your briefing says: act first, then reply with one short line saying what you did. If it needs nothing from you, run no commands, say so in a few words and stop. Do not recap other work, and do not ask the operator anything you can decide yourself.\n";

/// How many finished jobs a manager is shown. Its session lives as long as the project does, so the
/// full list only grows; the ones still going and the latest to finish are what a turn can act on, and
/// `tendril job list` has the rest.
const MANAGER_FINISHED_JOBS: usize = 8;

/// The jobs block for a manager: everything still going, the last few that finished, and no advice on
/// what to say about them.
fn manager_jobs_section(jobs: &[ChatSpawnedJob]) -> String {
    let (finished, going): (Vec<&ChatSpawnedJob>, Vec<&ChatSpawnedJob>) =
        jobs.iter().partition(|j| j.is_completed() || j.is_failed());
    if going.is_empty() && finished.is_empty() {
        return String::new();
    }
    let mut out = String::from("# Plan jobs you started\n");
    if going.is_empty() {
        out.push_str("None are running.\n");
    } else {
        out.push_str("Still going:\n");
        going.iter().for_each(|j| out.push_str(&j.prompt_line()));
    }
    if !finished.is_empty() {
        let skipped = finished.len().saturating_sub(MANAGER_FINISHED_JOBS);
        out.push_str("Latest to finish");
        if skipped > 0 {
            out.push_str(&format!(" ({skipped} older ones not shown)"));
        }
        out.push_str(":\n");
        finished[skipped..].iter().for_each(|j| out.push_str(&j.prompt_line()));
    }
    out.push_str("Missions are not listed here: `tendril mission list` has them.\n---\n\n");
    out
}

/// One job this chat session set running, as the prompt describes it. The fields are the ones V1 lists
/// (`ChatExecutionService`'s spawned-jobs block); everything else on a job is noise in a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSpawnedJob {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub plan_id: Option<String>,
    pub plan_title: Option<String>,
    pub status_message: Option<String>,
}

impl ChatSpawnedJob {
    fn prompt_line(&self) -> String {
        let plan = match (&self.plan_id, &self.plan_title) {
            (Some(id), Some(title)) if !id.is_empty() => format!(" | Plan: {} ({})", id, title),
            (Some(id), None) if !id.is_empty() => format!(" | Plan: {}", id),
            _ => String::new(),
        };
        let message = match &self.status_message {
            Some(message) if !message.trim().is_empty() => format!(" | Message: {}", message),
            _ => String::new(),
        };
        format!(
            "- Job {}: {} | Status: {}{}{}\n",
            self.id, self.job_type, self.status, plan, message
        )
    }

    fn is_completed(&self) -> bool {
        self.status.eq_ignore_ascii_case("Completed")
    }

    fn is_failed(&self) -> bool {
        ["Failed", "Timeout", "Stopped"]
            .iter()
            .any(|s| self.status.eq_ignore_ascii_case(s))
    }
}

/// Which of V1's three closing sentences the jobs section ends with.
enum JobRollup {
    AllDone,
    AnyFailed,
    StillRunning,
}

impl JobRollup {
    fn of(jobs: &[ChatSpawnedJob]) -> Self {
        if jobs.iter().all(ChatSpawnedJob::is_completed) {
            // V1 checks "all done" before "any failed", so a set that finished with failures in it is
            // reported as finished — a failed job *is* done, and the failure is on its own line above.
            return Self::AllDone;
        }
        if jobs.iter().any(ChatSpawnedJob::is_failed) {
            return Self::AnyFailed;
        }
        Self::StillRunning
    }
}

/// Whether a turn's message is an event Tendril injected rather than something the user typed.
/// `ChatExecutionService` compares the role to `"system"` case-insensitively, and so does this.
pub fn is_event_role(role: &str) -> bool {
    role.trim().eq_ignore_ascii_case("system")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    /// An injected event is framed as one, and a session's jobs are context for whatever it is asked.
    /// The framing is the point: without it the agent answers the event as though the user had typed it.
    #[test]
    fn test_build_chat_agent_prompt_frames_events_and_lists_jobs() {
        let event = "[System Event] Job 00042 (ExecutePlan) completed.";
        let injected = build_chat_agent_prompt(&[], event, "sess-1", "system", &[]);
        assert!(
            injected.contains("# Current Event Notification"),
            "got: {}",
            injected
        );
        assert!(!injected.contains("# Current User Request"));
        assert!(injected.contains(event));
        // The instruction is what turns a notification into something to act on.
        assert!(injected.contains("Evaluate this completed job event."));
        assert!(
            injected.contains("advise the user with a concise summary and suggested next steps")
        );

        // The role check is case-insensitive, as `ChatExecutionService`'s is, and anything else is a
        // user request.
        assert!(is_event_role("system") && is_event_role("System") && is_event_role(" SYSTEM "));
        for role in ["user", "", "assistant", "systemic"] {
            assert!(!is_event_role(role), "role: {}", role);
        }
        assert!(build_chat_agent_prompt(&[], "hi", "s", "assistant", &[])
            .contains("# Current User Request"));

        let job = |id: &str, status: &str| ChatSpawnedJob {
            id: id.to_string(),
            job_type: "ExecutePlan".to_string(),
            status: status.to_string(),
            plan_id: Some("00042".to_string()),
            plan_title: Some("Port the chat".to_string()),
            status_message: None,
        };

        // Every job is listed with its plan, and the closing sentence reads the set as a whole.
        let all_done =
            build_chat_agent_prompt(&[], "now what?", "s", "user", &[job("1", "Completed")]);
        assert!(all_done.contains("# Jobs Spawned in this Chat Session"));
        assert!(all_done
            .contains("- Job 1: ExecutePlan | Status: Completed | Plan: 00042 (Port the chat)"));
        assert!(all_done.contains("All spawned jobs have completed."));

        let failed = build_chat_agent_prompt(
            &[],
            "now what?",
            "s",
            "user",
            &[job("1", "Completed"), job("2", "Failed")],
        );
        assert!(
            failed.contains("Some spawned jobs failed"),
            "got: {}",
            failed
        );

        let running =
            build_chat_agent_prompt(&[], "now what?", "s", "user", &[job("1", "Running")]);
        assert!(
            running.contains("still running or pending"),
            "got: {}",
            running
        );

        // A status message is carried; no jobs means no section at all.
        let mut with_message = job("3", "Failed");
        with_message.status_message = Some("verification failed".to_string());
        let detailed = build_chat_agent_prompt(&[], "?", "s", "user", &[with_message]);
        assert!(
            detailed.contains("| Message: verification failed"),
            "got: {}",
            detailed
        );
        assert!(!build_chat_agent_prompt(&[], "?", "s", "user", &[])
            .contains("Jobs Spawned in this Chat Session"));
    }

    /// A manager is told to act and report, never to advise and ask, and is not handed every job it
    /// ever started.
    #[test]
    fn a_manager_gets_its_own_framing_and_a_bounded_jobs_list() {
        let job = |id: usize, status: &str| ChatSpawnedJob {
            id: format!("{id:05}"),
            job_type: "ExecutePlan".to_string(),
            status: status.to_string(),
            plan_id: None,
            plan_title: None,
            status_message: None,
        };
        let mut jobs: Vec<ChatSpawnedJob> = (1..=20).map(|i| job(i, if i % 5 == 0 { "Failed" } else { "Completed" })).collect();
        jobs.push(job(21, "Running"));

        let woken = build_chat_agent_prompt(&[], "Patrol: ...", "manager-acme", "system", &jobs);
        assert!(woken.contains("# Current Event Notification"));
        assert!(woken.contains("act first, then reply with one short line"), "{woken}");
        for other_chats in ["Evaluate this completed job event", "advise the user", "guide the user", "Guide the user"] {
            assert!(!woken.contains(other_chats), "{other_chats}: {woken}");
        }
        assert!(woken.contains("Still going:\n- Job 00021"), "{woken}");
        assert!(woken.contains("(12 older ones not shown)"), "{woken}");
        assert!(!woken.contains("Job 00012:") && woken.contains("Job 00013:") && woken.contains("Job 00020:"), "{woken}");

        // Asked by the operator, it is a request like any other; with no jobs there is no block.
        let asked = build_chat_agent_prompt(&[], "where are we?", "manager-acme", "user", &[]);
        assert!(asked.contains("# Current User Request") && !asked.contains("Plan jobs you started"));
        // Any other chat keeps V1's wording.
        let plain = build_chat_agent_prompt(&[], "done", "0d1f-some-chat", "system", &jobs);
        assert!(plain.contains("Evaluate this completed job event") && plain.contains("# Jobs Spawned in this Chat Session"));
    }

    #[test]
    fn a_manager_is_replayed_its_briefing_and_only_its_latest_messages() {
        let msg = |role: &str, content: String| ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: role.to_string(),
            content,
            timestamp: Utc::now(),
            agent_id: None,
            model_id: None,
            raw_stream: None,
            effort: None,
        };
        let mut history = vec![msg("system", "# You are the Factory Manager for project \"Acme\"\nrules".into())];
        for i in 0..40 {
            history.push(msg("system", format!("event {i} {}", "x".repeat(900))));
            history.push(msg("assistant", format!("reply {i}")));
        }
        let state = "# Where the project stands\nNothing is in flight.\n---\n\n";
        let text = build_chat_agent_prompt_with_state(&history, "Patrol", "manager-acme", "system", &[], Some(state));
        assert!(text.contains("### Your briefing\n# You are the Factory Manager"), "the briefing is always there");
        assert!(text.contains("(56 earlier messages are not shown"), "{}", &text[..600]);
        assert!(!text.contains("reply 27\n") && text.contains("reply 28\n") && text.contains("reply 39\n"));
        assert!(text.contains("(clipped; already handled)") && !text.contains(&"x".repeat(500)));
        // What is current comes after the history and just before the event.
        let (h, s, e) = (text.find("reply 39").unwrap(), text.find("# Where the project stands").unwrap(), text.find("# Current Event").unwrap());
        assert!(h < s && s < e);
        assert!(text.contains("run no commands"));
        // Any other chat is replayed whole.
        let plain = build_chat_agent_prompt(&history, "hi", "0d1f-chat", "user", &[]);
        assert!(plain.contains("reply 0\n") && plain.contains(&"x".repeat(900)));
    }

    #[test]
    fn test_build_chat_agent_prompt_replays_history() {
        let msg = |role: &str, content: &str| ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: role.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            agent_id: None,
            model_id: None,
            raw_stream: None,
            effort: None,
        };

        let first = build_chat_agent_prompt(&[], "what is broken?", "sess-1", "user", &[]);
        assert!(!first.contains("Previous Conversation"));
        assert!(first.contains("Chat Session ID: sess-1"));
        assert!(first.contains("--chat-session sess-1"));
        assert!(first.contains("what is broken?"));

        let history = vec![
            msg("user", "what is broken?"),
            msg("assistant", "the chat path"),
            // An assistant stub a turn never filled in has nothing to replay.
            msg("assistant", "   "),
        ];
        let second = build_chat_agent_prompt(&history, "fix it", "sess-1", "user", &[]);
        assert!(second.contains("# Previous Conversation Discussion History"));
        assert!(second.contains("### User\nwhat is broken?"));
        assert!(second.contains("### Assistant\nthe chat path"));
        assert!(second.contains("fix it"));
        assert_eq!(
            second.matches("### Assistant").count(),
            1,
            "an empty message must not be replayed"
        );
    }
}
