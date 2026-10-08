//! What a project needs from the person: the one definition shared by the app's badges and the public
//! API, so "needs you" means the same thing everywhere.
//!
//! Only two things count. A mission waiting on the operator (awaiting approval, or paused), and a
//! manager that asked a question and has not been answered. Missions and tasks finishing do not: that is
//! the manager's to handle.

use crate::state::AppState;
use tendril_core::chat::manager_brief::manager_session_id;
use tendril_core::missions::model::MissionState;
use tendril_core::missions::store::list_missions;

/// How long a paused or awaiting-approval mission is the manager's alone to sort out. The daemon wakes
/// the manager when it happens and again after 20 minutes; only a mission still stuck after this is
/// something the operator is told about. A project with no manager at all has no grace.
pub const MANAGER_GRACE_MINUTES: i64 = 30;

/// Whether a mission waiting in `state` since `updated` now needs the operator.
pub fn mission_needs_operator(
    state: MissionState,
    updated: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    has_manager: bool,
) -> bool {
    matches!(state, MissionState::AwaitingApproval | MissionState::Paused)
        && (!has_manager || now - updated >= chrono::Duration::minutes(MANAGER_GRACE_MINUTES))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attention {
    /// Missions waiting for approval or paused.
    pub waiting_missions: usize,
    /// The manager's latest message is a question and nobody has replied yet.
    pub asked: bool,
}

impl Attention {
    /// How many things are waiting on a person.
    pub fn count(&self) -> usize {
        self.waiting_missions + usize::from(self.asked)
    }
}

/// Whether the conversation ends on the manager asking something: its last visible message is its own,
/// the last paragraph of it is a question, and no turn is running to follow it up.
pub fn ends_on_question(session: &tendril_core::chat::ChatSession, generating: bool) -> bool {
    if generating {
        return false;
    }
    let Some(last) = session
        .messages
        .iter()
        .rev()
        .find(|m| (m.role == "user" || m.role == "assistant") && !m.content.trim().is_empty())
    else {
        return false;
    };
    last.role == "assistant" && crate::push::last_paragraph(&last.content).ends_with('?')
}

pub async fn for_project(state: &AppState, project: &str) -> Attention {
    let id = manager_session_id(project);
    let generating = state.chat_manager.is_generating(&id).await;
    let session = state.chat_manager.get_session(&id).await.ok();
    let has_manager = session.is_some();
    let asked = session.as_ref().map(|s| ends_on_question(s, generating)).unwrap_or(false);
    let now = chrono::Utc::now();
    let waiting_missions = list_missions(&state.mission_driver.paths().missions_dir)
        .into_iter()
        .filter(|f| f.mission.project.eq_ignore_ascii_case(project))
        .filter(|f| mission_needs_operator(f.mission.state, f.mission.updated, now, has_manager))
        .count();
    Attention { waiting_missions, asked }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tendril_core::chat::{ChatMessage, ChatSession};

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: role.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            agent_id: None,
            model_id: None,
            raw_stream: None,
            effort: None,
        }
    }

    fn session(messages: Vec<ChatMessage>) -> ChatSession {
        ChatSession {
            id: "manager-x".into(),
            title: "Manager".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            agent_id: "claude".into(),
            model_id: "opus".into(),
            messages,
            effort: None,
            spawned_job_ids: vec![],
            plan_folder_name: None,
        }
    }

    #[test]
    fn an_unanswered_question_needs_you() {
        let s = session(vec![message("system", "# briefing"), message("user", "go"), message("assistant", "Done the API.\n\nShall I ship it?")]);
        assert!(ends_on_question(&s, false));
    }

    #[test]
    fn answering_or_a_running_turn_clears_it() {
        let answered = session(vec![message("assistant", "Ship it?"), message("user", "yes")]);
        assert!(!ends_on_question(&answered, false));
        let busy = session(vec![message("assistant", "Ship it?")]);
        assert!(!ends_on_question(&busy, true), "a manager mid-turn is not waiting");
    }

    #[test]
    fn statements_and_empty_conversations_do_not() {
        assert!(!ends_on_question(&session(vec![message("assistant", "All done.")]), false));
        assert!(!ends_on_question(&session(vec![]), false));
        // A question earlier in the reply does not count; only how it ends.
        assert!(!ends_on_question(&session(vec![message("assistant", "Why did it fail? A flaky test. Re-ran it.")]), false));
    }

    #[test]
    fn a_stuck_mission_is_the_managers_for_a_while_before_it_is_yours() {
        use MissionState::*;
        let now = chrono::Utc::now();
        let ago = |m: i64| now - chrono::Duration::minutes(m);
        assert!(!mission_needs_operator(Paused, ago(5), now, true), "the manager is still on it");
        assert!(mission_needs_operator(Paused, ago(31), now, true), "it had its chance");
        assert!(mission_needs_operator(AwaitingApproval, ago(45), now, true));
        assert!(mission_needs_operator(Paused, ago(1), now, false), "no manager, no grace");
        assert!(!mission_needs_operator(Running, ago(300), now, true));
        assert!(!mission_needs_operator(Review, ago(300), now, true));
    }

    #[test]
    fn the_count_adds_missions_and_a_question() {
        assert_eq!(Attention { waiting_missions: 2, asked: true }.count(), 3);
        assert_eq!(Attention::default().count(), 0);
    }
}
