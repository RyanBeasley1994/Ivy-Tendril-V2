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
    let waiting_missions = list_missions(&state.mission_driver.paths().missions_dir)
        .into_iter()
        .filter(|f| f.mission.project.eq_ignore_ascii_case(project))
        .filter(|f| matches!(f.mission.state, MissionState::AwaitingApproval | MissionState::Paused))
        .count();
    let id = manager_session_id(project);
    let generating = state.chat_manager.is_generating(&id).await;
    let asked = match state.chat_manager.get_session(&id).await {
        Ok(session) => ends_on_question(&session, generating),
        Err(_) => false,
    };
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
    fn the_count_adds_missions_and_a_question() {
        assert_eq!(Attention { waiting_missions: 2, asked: true }.count(), 3);
        assert_eq!(Attention::default().count(), 0);
    }
}
