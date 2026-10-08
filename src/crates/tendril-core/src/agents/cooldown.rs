//! Agents that are rate limited right now, and who covers for them until the limit resets.
//!
//! When an agent reports a rate or usage limit, the moment it clears is known (the message usually
//! says "try again in 23 minutes") or can be estimated. Until then, anything that would start on that
//! agent starts on the next one in the operator's fallback chain instead. Nothing is rewritten: the
//! role, the mission and the chat session all still name the agent the operator chose, so the moment the
//! cooldown passes, new work is back on it with no one having to switch it back.
//!
//! The cooldowns live in memory. A daemon that restarts forgets them, tries the primary once, is told
//! again, and records it again.

use crate::missions::model::RoleAgent;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Mutex;

static COOLDOWNS: Mutex<Option<HashMap<String, DateTime<Utc>>>> = Mutex::new(None);

fn key(agent: &str) -> String {
    agent.trim().to_ascii_lowercase()
}

fn with_map<T>(f: impl FnOnce(&mut HashMap<String, DateTime<Utc>>) -> T) -> T {
    let mut guard = COOLDOWNS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Records that `agent` is rate limited until `until` (the later of this and any earlier record).
pub fn note_rate_limited(agent: &str, until: DateTime<Utc>) {
    if agent.trim().is_empty() {
        return;
    }
    with_map(|m| {
        let entry = m.entry(key(agent)).or_insert(until);
        if until > *entry {
            *entry = until;
        }
    });
}

/// When `agent`'s limit clears, if it is limited at `now`.
pub fn limited_until(agent: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    with_map(|m| m.get(&key(agent)).copied().filter(|until| *until > now))
}

/// Every agent limited at `now`, soonest to clear first. Cleared ones are forgotten.
pub fn active(now: DateTime<Utc>) -> Vec<(String, DateTime<Utc>)> {
    with_map(|m| {
        m.retain(|_, until| *until > now);
        let mut list: Vec<_> = m.iter().map(|(a, u)| (a.clone(), *u)).collect();
        list.sort_by_key(|(_, u)| *u);
        list
    })
}

/// Forgets an agent's cooldown (it was noted in error, or the operator says it is back).
pub fn clear(agent: &str) {
    with_map(|m| {
        m.remove(&key(agent));
    });
}

/// The engine new work should start on.
///
/// `planned` is what the role asks for (`None` means the configured default agent, named by
/// `default_agent`). If that agent is not limited, it is returned untouched. If it is, the first agent in
/// `fallbacks` that is not itself limited takes over. With no usable fallback the planned engine is
/// returned anyway: starting on a limited agent and waiting is better than starting nothing.
pub fn resolve(
    fallbacks: &[RoleAgent],
    planned: Option<&RoleAgent>,
    default_agent: &str,
    now: DateTime<Utc>,
) -> Option<RoleAgent> {
    let current = planned.map(|p| p.agent.as_str()).unwrap_or(default_agent);
    if limited_until(current, now).is_none() {
        return planned.cloned();
    }
    fallbacks
        .iter()
        .find(|f| !f.agent.eq_ignore_ascii_case(current) && limited_until(&f.agent, now).is_none())
        .cloned()
        .or_else(|| planned.cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn role(agent: &str) -> RoleAgent {
        RoleAgent { agent: agent.into(), model: None, effort: None }
    }

    // The map is process-wide, so each test uses agent names no other test touches.

    #[test]
    fn a_limited_agent_is_covered_for_until_it_resets_and_then_is_back() {
        let now = Utc::now();
        let chain = [role("cd-fallback-a")];
        let primary = role("cd-primary-a");
        // Not limited: untouched.
        assert_eq!(resolve(&chain, Some(&primary), "x", now).unwrap().agent, "cd-primary-a");
        // Limited for 20 minutes: the fallback takes over.
        note_rate_limited("cd-primary-a", now + Duration::minutes(20));
        assert_eq!(resolve(&chain, Some(&primary), "x", now).unwrap().agent, "cd-fallback-a");
        assert_eq!(resolve(&chain, Some(&primary), "x", now + Duration::minutes(19)).unwrap().agent, "cd-fallback-a");
        // The limit resets: back on the primary, with nothing having been switched back.
        assert_eq!(resolve(&chain, Some(&primary), "x", now + Duration::minutes(21)).unwrap().agent, "cd-primary-a");
    }

    #[test]
    fn the_default_agent_is_covered_too_and_a_limited_fallback_is_skipped() {
        let now = Utc::now();
        let chain = [role("cd-fallback-b1"), role("cd-fallback-b2")];
        note_rate_limited("cd-default-b", now + Duration::minutes(10));
        // A role with no engine of its own runs on the default agent, which is limited.
        assert_eq!(resolve(&chain, None, "cd-default-b", now).unwrap().agent, "cd-fallback-b1");
        // The first fallback is limited as well: the next one covers.
        note_rate_limited("cd-fallback-b1", now + Duration::minutes(10));
        assert_eq!(resolve(&chain, None, "cd-default-b", now).unwrap().agent, "cd-fallback-b2");
    }

    #[test]
    fn with_nowhere_to_go_the_planned_engine_is_kept_and_a_chain_never_points_at_itself() {
        let now = Utc::now();
        let primary = role("cd-primary-c");
        note_rate_limited("cd-primary-c", now + Duration::minutes(5));
        assert_eq!(resolve(&[], Some(&primary), "x", now).unwrap().agent, "cd-primary-c");
        assert_eq!(resolve(&[role("CD-PRIMARY-C")], Some(&primary), "x", now).unwrap().agent, "cd-primary-c");
        // An unset role on an unlimited default stays unset.
        assert!(resolve(&[role("cd-fallback-c")], None, "cd-default-c", now).is_none());
    }

    #[test]
    fn the_later_reset_wins_and_cleared_agents_are_forgotten() {
        let now = Utc::now();
        note_rate_limited("cd-d", now + Duration::minutes(5));
        note_rate_limited("cd-d", now + Duration::minutes(30));
        note_rate_limited("cd-d", now + Duration::minutes(1));
        assert_eq!(limited_until("cd-d", now), Some(now + Duration::minutes(30)));
        assert!(active(now).iter().any(|(a, _)| a == "cd-d"));
        clear("cd-d");
        assert!(limited_until("cd-d", now).is_none());
        assert!(active(now).iter().all(|(a, _)| a != "cd-d"));
    }
}
