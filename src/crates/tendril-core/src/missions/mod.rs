//! Missions: a goal broken into milestones by an AI orchestrator, run one after another on a shared
//! branch, each judged against its acceptance criteria, and validated as a whole before it becomes a
//! single pull request.
//!
//! * [`model`] — `mission.yaml` and the link a plan keeps back to its mission.
//! * [`store`] — reading and writing mission folders.
//! * [`service`] — the file-level operations the CLI and the server share.
//! * [`driver`] — the reconciler that starts each step's job and applies the orchestrator's decisions.
//! * [`git`] — the mission branch.

pub mod driver;
pub mod git;
pub mod model;
pub mod service;
pub mod store;

use crate::missions::model::{mission_link, MissionRole};
use crate::models::PlanYaml;
use std::path::Path;

/// Why an integration plan may not run yet, or `None` when it may (or is not one).
///
/// Used by the dependency gate: the integration plan stays `Blocked` until its mission is in Review,
/// so neither an operator nor the unblock pass can execute it underneath the mission. A mission file
/// that cannot be read releases the plan rather than stranding it.
pub fn integration_block_reason(plan: &PlanYaml) -> Option<String> {
    let link = mission_link(plan).filter(|l| l.role == MissionRole::Integration)?;
    let folder = Path::new(&link.folder);
    let mission = store::read_mission(folder).ok()?;
    if mission.state.releases_integration_plan() {
        return None;
    }
    let id = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&link.folder);
    Some(format!("Waiting on mission {} ({})", id, mission.state))
}
