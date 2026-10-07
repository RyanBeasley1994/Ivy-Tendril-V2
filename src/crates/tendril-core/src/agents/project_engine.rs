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
