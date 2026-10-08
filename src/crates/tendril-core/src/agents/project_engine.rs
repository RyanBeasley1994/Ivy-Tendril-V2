//! A project's engines: which coding agent and model each role in that project runs on.
//!
//! Roles are set one at a time and changed whenever the operator likes (for instance to move a project
//! from Claude to a local model while Claude is rate-limited):
//!
//! * `manager`: the project's manager chat.
//! * `planner`, `worker`, `judge`, `validator`: the four roles of a mission, and the plan jobs that
//!   match them (`planner` writes plans, `worker` executes them).
//!
//! A role left unset runs on the configured default agent. Stored in
//! `<TENDRIL_HOME>/Projects/<Project>/engine.json`, so it is read the same way by the server, the CLI
//! and the mission service, with no daemon in the loop.

use crate::config::get_project_root_dir;
use crate::missions::model::{MissionAgents, RoleAgent};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The roles an engine can be set for, in the order the UI lists them.
pub const ROLES: [&str; 5] = ["manager", "planner", "worker", "judge", "validator"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEngine {
    #[serde(default)]
    pub roles: BTreeMap<String, RoleAgent>,
}

/// The engines that apply to every project, and what to switch to when one hits a rate limit.
///
/// Stored in `<TENDRIL_HOME>/engine.json`. A project's own engine (above) overrides a role here; a role
/// neither sets runs on the configured default agent. `fallbacks` is an ordered chain: when the agent a
/// piece of work is running on is rate limited, the next agent in the chain takes over instead of the work
/// waiting out the limit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalEngine {
    #[serde(default)]
    pub roles: BTreeMap<String, RoleAgent>,
    #[serde(default)]
    pub fallbacks: Vec<RoleAgent>,
}

fn global_path(tendril_home: &Path) -> PathBuf {
    tendril_home.join("engine.json")
}

impl GlobalEngine {
    pub fn load(tendril_home: &Path) -> Self {
        std::fs::read_to_string(global_path(tendril_home))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, tendril_home: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(tendril_home)?;
        std::fs::write(global_path(tendril_home), serde_json::to_vec_pretty(self).unwrap_or_default())
    }

    /// Sets (`Some`) or clears (`None`) one role. Unknown role names are ignored.
    pub fn set(&mut self, role: &str, value: Option<RoleAgent>) {
        if !ROLES.contains(&role) {
            return;
        }
        match value.filter(|v| !v.agent.trim().is_empty()) {
            Some(v) => {
                self.roles.insert(role.to_string(), v);
            }
            None => {
                self.roles.remove(role);
            }
        }
    }

    /// Replaces the fallback chain. Blank agents and repeats of an agent already in the chain are dropped.
    pub fn set_fallbacks(&mut self, chain: Vec<RoleAgent>) {
        let mut out: Vec<RoleAgent> = Vec::new();
        for item in chain.into_iter().filter(|a| !a.agent.trim().is_empty()) {
            if !out.iter().any(|existing| same_engine(existing, &item)) {
                out.push(item);
            }
        }
        self.fallbacks = out;
    }
}

/// Whether two role agents are the same engine: the same agent running the same model.
fn same_engine(a: &RoleAgent, b: &RoleAgent) -> bool {
    a.agent.eq_ignore_ascii_case(&b.agent)
        && a.model.as_deref().unwrap_or_default().eq_ignore_ascii_case(b.model.as_deref().unwrap_or_default())
}

/// The agent to switch to when `current` is rate limited: the next one in the chain after `current`.
///
/// `primary` is what the role would normally run on (it may be unset, meaning the configured default,
/// which is then not part of the chain). From the primary the next is the first fallback; from a fallback
/// the one after it; from an agent that is in neither, the first fallback that is not `current`. Nothing
/// is returned once the chain is spent, and the work waits out the limit as it always did.
pub fn next_fallback(fallbacks: &[RoleAgent], primary: Option<&RoleAgent>, current: &RoleAgent) -> Option<RoleAgent> {
    if let Some(at) = fallbacks.iter().position(|f| same_engine(f, current)) {
        return fallbacks.get(at + 1).cloned();
    }
    let _ = primary; // The primary is "before" the chain, so leaving it means the chain's first entry.
    fallbacks.iter().find(|f| !f.agent.eq_ignore_ascii_case(&current.agent)).cloned()
}

/// What `engine.json` looked like before roles: one engine for everything.
#[derive(Deserialize)]
struct LegacyEngine {
    agent: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
}

fn engine_path(tendril_home: &Path, project: &str) -> PathBuf {
    get_project_root_dir(tendril_home, project).join("engine.json")
}

impl ProjectEngine {
    pub fn load(tendril_home: &Path, project: &str) -> Self {
        let Ok(raw) = std::fs::read_to_string(engine_path(tendril_home, project)) else {
            return Self::default();
        };
        if let Ok(engine) = serde_json::from_str::<ProjectEngine>(&raw) {
            if !engine.roles.is_empty() {
                return engine;
            }
        }
        // The single-engine form applies to every role.
        match serde_json::from_str::<LegacyEngine>(&raw) {
            Ok(legacy) if !legacy.agent.trim().is_empty() => Self {
                roles: ROLES
                    .iter()
                    .map(|r| {
                        (
                            (*r).to_string(),
                            RoleAgent {
                                agent: legacy.agent.clone(),
                                model: legacy.model.clone(),
                                effort: legacy.effort.clone(),
                            },
                        )
                    })
                    .collect(),
            },
            _ => Self::default(),
        }
    }

    /// What actually applies to `project`: the global engine's roles, overridden by the project's own.
    /// This is for deciding what to run; edit a project's own settings through [`ProjectEngine::load`].
    pub fn load_effective(tendril_home: &Path, project: &str) -> Self {
        let mut roles = GlobalEngine::load(tendril_home).roles;
        roles.extend(Self::load(tendril_home, project).roles);
        Self { roles }
    }

    pub fn save(&self, tendril_home: &Path, project: &str) -> std::io::Result<()> {
        let path = engine_path(tendril_home, project);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default())
    }

    pub fn role(&self, role: &str) -> Option<&RoleAgent> {
        self.roles.get(role)
    }

    /// Sets (`Some`) or clears (`None`) one role. Unknown role names are ignored.
    pub fn set(&mut self, role: &str, value: Option<RoleAgent>) {
        if !ROLES.contains(&role) {
            return;
        }
        match value.filter(|v| !v.agent.trim().is_empty()) {
            Some(v) => {
                self.roles.insert(role.to_string(), v);
            }
            None => {
                self.roles.remove(role);
            }
        }
    }

    /// The four mission roles as the mission file stores them.
    pub fn mission_agents(&self) -> MissionAgents {
        MissionAgents {
            planner: self.role("planner").cloned(),
            worker: self.role("worker").cloned(),
            judge: self.role("judge").cloned(),
            validator: self.role("validator").cloned(),
        }
    }

    /// The engine a plan job should default to: plan-writing jobs use `planner`, the rest `worker`.
    pub fn for_job_type(&self, job_type: &str) -> Option<&RoleAgent> {
        match job_type {
            "CreatePlan" | "ExpandPlan" | "UpdatePlan" | "SplitPlan" => self.role("planner"),
            "ExecutePlan" | "RetryPlan" | "CreatePr" => self.role("worker"),
            _ => None,
        }
    }
}

/// `target` with every role the project's engine sets filled in where `target` leaves it unset.
pub fn fill_unset_mission_roles(target: MissionAgents, engine: &ProjectEngine) -> MissionAgents {
    let defaults = engine.mission_agents();
    MissionAgents {
        planner: target.planner.or(defaults.planner),
        worker: target.worker.or(defaults.worker),
        judge: target.judge.or(defaults.judge),
        validator: target.validator.or(defaults.validator),
    }
}

/// `current` with the roles `engine` sets overwritten: how a live mission follows a switch.
pub fn apply_mission_roles(current: MissionAgents, changed: &[&str], engine: &ProjectEngine) -> MissionAgents {
    let mut out = current;
    for role in changed {
        let value = engine.role(role).cloned();
        match *role {
            "planner" => out.planner = value,
            "worker" => out.worker = value,
            "judge" => out.judge = value,
            "validator" => out.validator = value,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(agent: &str, model: Option<&str>) -> RoleAgent {
        RoleAgent { agent: agent.into(), model: model.map(str::to_string), effort: None }
    }

    #[test]
    fn roles_are_set_and_cleared_one_by_one() {
        let mut e = ProjectEngine::default();
        e.set("worker", Some(role("codex", Some("qwen3"))));
        e.set("judge", Some(role("claude", None)));
        e.set("nonsense", Some(role("claude", None)));
        assert_eq!(e.roles.len(), 2, "an unknown role is ignored");
        e.set("worker", None);
        assert!(e.role("worker").is_none());
        assert!(e.role("judge").is_some());
        e.set("judge", Some(role("  ", None)));
        assert!(e.role("judge").is_none(), "a blank agent clears the role");
    }

    #[test]
    fn new_missions_inherit_only_the_roles_the_request_left_unset() {
        let mut e = ProjectEngine::default();
        e.set("worker", Some(role("codex", Some("local"))));
        e.set("planner", Some(role("claude", None)));
        let requested = MissionAgents { planner: Some(role("gemini", None)), ..Default::default() };
        let out = fill_unset_mission_roles(requested, &e);
        assert_eq!(out.planner.unwrap().agent, "gemini", "an explicit choice wins");
        assert_eq!(out.worker.unwrap().agent, "codex");
        assert!(out.judge.is_none());
    }

    #[test]
    fn a_live_mission_only_changes_the_roles_that_were_touched() {
        let mut e = ProjectEngine::default();
        e.set("worker", Some(role("codex", None)));
        let current = MissionAgents {
            worker: Some(role("claude", None)),
            judge: Some(role("claude", None)),
            ..Default::default()
        };
        let out = apply_mission_roles(current, &["worker"], &e);
        assert_eq!(out.worker.unwrap().agent, "codex");
        assert_eq!(out.judge.unwrap().agent, "claude", "untouched roles stay as they were");
    }

    #[test]
    fn jobs_pick_planner_or_worker_by_what_they_do() {
        let mut e = ProjectEngine::default();
        e.set("planner", Some(role("claude", None)));
        e.set("worker", Some(role("codex", None)));
        assert_eq!(e.for_job_type("CreatePlan").unwrap().agent, "claude");
        assert_eq!(e.for_job_type("ExecutePlan").unwrap().agent, "codex");
        assert_eq!(e.for_job_type("RetryPlan").unwrap().agent, "codex");
        assert!(e.for_job_type("SyncRepo").is_none());
    }

    #[test]
    fn a_projects_own_engine_overrides_the_global_one_role_by_role() {
        let home = std::env::temp_dir().join(format!("tendril-engine-{}", uuid::Uuid::new_v4()));
        let mut global = GlobalEngine::default();
        global.set("worker", Some(role("claude", Some("opus"))));
        global.set("judge", Some(role("claude", None)));
        global.save(&home).unwrap();
        let mut mine = ProjectEngine::default();
        mine.set("worker", Some(role("codex", Some("local"))));
        mine.save(&home, "Acme").unwrap();

        let effective = ProjectEngine::load_effective(&home, "Acme");
        assert_eq!(effective.role("worker").unwrap().agent, "codex", "the project wins");
        assert_eq!(effective.role("judge").unwrap().agent, "claude", "the rest is inherited");
        // A project with nothing of its own gets the global engine whole.
        assert_eq!(ProjectEngine::load_effective(&home, "Other").role("worker").unwrap().agent, "claude");
        // And editing a project never copies inherited roles into its own file.
        assert_eq!(ProjectEngine::load(&home, "Acme").roles.len(), 1);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_fallback_chain_is_deduplicated_and_walked_in_order() {
        let mut g = GlobalEngine::default();
        g.set_fallbacks(vec![role("codex", Some("local")), role(" ", None), role("CODEX", Some("local")), role("gemini", None)]);
        assert_eq!(g.fallbacks.len(), 2, "blank and repeated entries are dropped");

        let chain = g.fallbacks.clone();
        let claude = role("claude", None);
        // From the primary, the first fallback; then each next one; then nothing left.
        assert_eq!(next_fallback(&chain, Some(&claude), &claude).unwrap().agent, "codex");
        assert_eq!(next_fallback(&chain, Some(&claude), &role("codex", Some("local"))).unwrap().agent, "gemini");
        assert!(next_fallback(&chain, Some(&claude), &role("gemini", None)).is_none(), "the chain is spent");
        // An unset primary (the configured default) behaves the same.
        assert_eq!(next_fallback(&chain, None, &claude).unwrap().agent, "codex");
        // Never "falls back" onto the agent that just failed, and an empty chain has nowhere to go.
        assert!(next_fallback(&[role("claude", None)], None, &claude).is_none());
        assert!(next_fallback(&[], None, &claude).is_none());
    }

    #[test]
    fn the_old_single_engine_file_still_reads_as_every_role() {
        let home = std::env::temp_dir().join(format!("tendril-engine-{}", uuid::Uuid::new_v4()));
        let path = engine_path(&home, "Acme");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"agent":"codex","model":"qwen3"}"#).unwrap();
        let e = ProjectEngine::load(&home, "Acme");
        assert_eq!(e.roles.len(), ROLES.len());
        assert_eq!(e.role("judge").unwrap().model.as_deref(), Some("qwen3"));
        let _ = std::fs::remove_dir_all(home);
    }
}
