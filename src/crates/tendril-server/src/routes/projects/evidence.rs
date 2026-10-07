//! `GET /api/projects/:name/evidence`: every screenshot and recording a project's workers have attached,
//! across all of its plans, newest first. The project page's Artifacts tab reads this.
//!
//! Evidence lives with the plan it was recorded against (`Artifacts/` plus a manifest). A plan that
//! belongs to a mission is labelled with the mission and milestone; any other plan of the project is
//! labelled with its own title, so work done outside missions shows up too.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tendril_core::config::load_config;
use tendril_core::missions::store::list_missions;
use tendril_core::plans::evidence::list_evidence;

/// The most items one response carries: a project's gallery, not its whole history.
const MAX_ITEMS: usize = 400;

pub async fn project_evidence(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let settings = match load_config(&state.config_path) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to load config: {e}") })))
                .into_response()
        }
    };
    let Some(project) = settings.projects.iter().find(|p| p.name.eq_ignore_ascii_case(&name)) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Project '{name}' not found") }))).into_response();
    };
    let project_name = project.name.clone();
    let plans_dir = state.plans_dir.clone();
    let missions_dir = state.mission_driver.paths().missions_dir.clone();

    let result = tokio::task::spawn_blocking(move || {
        // Which mission and milestone each plan folder belongs to.
        let mut owner: HashMap<String, (String, String, Option<String>, String)> = HashMap::new();
        for f in list_missions(&missions_dir)
            .into_iter()
            .filter(|f| f.mission.project.eq_ignore_ascii_case(&project_name))
        {
            for ms in &f.mission.milestones {
                if let Some(plan) = &ms.plan {
                    owner.insert(plan.clone(), (f.id.clone(), f.mission.title.clone(), Some(ms.id.clone()), ms.title.clone()));
                }
            }
            if let Some(plan) = &f.mission.integration_plan {
                owner.insert(plan.clone(), (f.id.clone(), f.mission.title.clone(), None, "Final result".to_string()));
            }
        }

        let mut groups = Vec::new();
        let Ok(entries) = std::fs::read_dir(&plans_dir) else { return Vec::new() };
        for entry in entries.flatten() {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let items = list_evidence(&folder);
            if items.is_empty() {
                continue;
            }
            let plan_folder = entry.file_name().to_string_lossy().to_string();
            let (mission, mission_title, milestone, title) = match owner.get(&plan_folder) {
                Some((id, mission_title, milestone, title)) => {
                    (Some(id.clone()), Some(mission_title.clone()), milestone.clone(), title.clone())
                }
                None => match tendril_core::plans::reader::read_plan_yaml(&folder) {
                    Ok((plan, _)) if plan.project.eq_ignore_ascii_case(&project_name) => (None, None, None, plan.title),
                    _ => continue,
                },
            };
            let newest = items.iter().filter_map(|i| i.at.clone()).max().unwrap_or_default();
            groups.push((
                newest.clone(),
                json!({
                    "missionId": mission,
                    "missionTitle": mission_title,
                    "milestoneId": milestone,
                    "title": title,
                    "planFolder": plan_folder,
                    "newest": newest,
                    "items": items,
                }),
            ));
        }
        // Newest first, then trimmed to the cap, oldest groups dropping off first.
        groups.sort_by(|a, b| b.0.cmp(&a.0));
        let mut total = 0usize;
        groups
            .into_iter()
            .filter_map(|(_, g)| {
                let n = g["items"].as_array().map(|a| a.len()).unwrap_or(0);
                if total >= MAX_ITEMS {
                    return None;
                }
                total += n;
                Some(g)
            })
            .collect::<Vec<_>>()
    })
    .await;

    match result {
        Ok(groups) => Json(json!({ "groups": groups })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}
