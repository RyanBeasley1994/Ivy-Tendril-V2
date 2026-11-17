//! More than one manager per project: the tabs above the manager chat.
//!
//! The project's main manager is always there and is never a tab here: it is the one the scheduler
//! patrols, the morning briefing comes from and Telegram's `/manager` talks to. Each tab the operator
//! opens with `+` is one more manager (`manager_brief::tab_session_id`): same briefing, same powers,
//! the same project snapshot every turn, its own conversation.
//!
//! What a tab's manager starts reports back to that tab: a worker's report, a wake-up it scheduled, a
//! pull request it asked to have watched ([`deliver_to`]). Closing a tab archives it; its conversation is
//! kept and it can be reopened. While it is closed, what was meant for it goes to the main manager, so
//! a report is never left in a conversation nobody is looking at.
//!
//! `<TendrilHome>/Projects/<Project>/manager-tabs.json` lists a project's tabs, open and closed.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tendril_core::chat::manager_brief::{base_manager_session_id, manager_session_id, tab_session_id};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: String,
    pub title: String,
    /// The chat session the tab's manager talks in.
    pub session_id: String,
    pub created_at: String,
    /// When it was closed. A closed tab keeps its conversation and can be reopened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
}

impl Tab {
    pub fn open(&self) -> bool {
        self.closed_at.is_none()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TabFile {
    tabs: Vec<Tab>,
}

static FILE_LOCK: Mutex<()> = Mutex::const_new(());

fn path(home: &Path, project: &str) -> PathBuf {
    tendril_core::config::get_project_root_dir(home, project).join("manager-tabs.json")
}

fn read(home: &Path, project: &str) -> TabFile {
    std::fs::read_to_string(path(home, project)).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

fn write(home: &Path, project: &str, file: &TabFile) -> Result<(), String> {
    let p = path(home, project);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(file).unwrap_or_default()).map_err(|e| e.to_string())
}

/// A project's tabs, open and closed, oldest first.
pub fn list(home: &Path, project: &str) -> Vec<Tab> {
    read(home, project).tabs
}

/// The name a new tab gets when it is not given one: "Manager 2", "Manager 3", ... (the main manager
/// being the first), never one already taken.
pub fn next_title(tabs: &[Tab]) -> String {
    (2..).map(|n| format!("Manager {n}")).find(|t| !tabs.iter().any(|tab| &tab.title == t)).unwrap_or_default()
}

/// Records a new tab. The caller opens its manager session.
pub async fn add(home: &Path, project: &str, title: Option<&str>) -> Result<Tab, String> {
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(home, project);
    let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect();
    let title = title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| next_title(&file.tabs));
    let tab = Tab {
        session_id: tab_session_id(project, &id),
        id,
        title,
        created_at: chrono::Utc::now().to_rfc3339(),
        closed_at: None,
    };
    file.tabs.push(tab.clone());
    write(home, project, &file)?;
    Ok(tab)
}

/// Changes one tab and returns it.
pub async fn update(home: &Path, project: &str, id: &str, edit: impl FnOnce(&mut Tab)) -> Result<Tab, String> {
    let _guard = FILE_LOCK.lock().await;
    let mut file = read(home, project);
    let tab = file.tabs.iter_mut().find(|t| t.id == id).ok_or_else(|| format!("{project} has no manager tab {id}"))?;
    edit(tab);
    let tab = tab.clone();
    write(home, project, &file)?;
    Ok(tab)
}

/// Whether `session` is one of `project`'s manager sessions (the main one, a tab or a ticket
/// conversation), so a caller cannot point a report at some other chat.
pub fn belongs_to(project: &str, session: &str) -> bool {
    base_manager_session_id(session) == manager_session_id(project)
}

/// Which session should hear about something a manager asked for: the one that asked, while it is still
/// there and (for a tab) open; otherwise the project's main manager. `None` when neither exists.
pub async fn deliver_to(state: &AppState, project: &str, asked_by: Option<&str>) -> Option<String> {
    let main = manager_session_id(project);
    if let Some(session) = asked_by.filter(|s| *s != main && belongs_to(project, s)) {
        let closed_tab = list(&state.tendril_home, project).iter().any(|t| t.session_id == session && !t.open());
        if !closed_tab && state.chat_manager.get_session(session).await.is_ok() {
            return Some(session.to_string());
        }
    }
    state.chat_manager.get_session(&main).await.ok()?;
    crate::routes::projects::refresh_briefing(state, project).await;
    Some(main)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tab_is_named_after_the_ones_already_there() {
        assert_eq!(next_title(&[]), "Manager 2");
        let tab = |title: &str| Tab { id: "x".into(), title: title.into(), session_id: "s".into(), created_at: String::new(), closed_at: None };
        assert_eq!(next_title(&[tab("Manager 2"), tab("Billing")]), "Manager 3");
        assert_eq!(next_title(&[tab("Manager 3")]), "Manager 2");
    }

    #[test]
    fn only_the_projects_own_managers_can_be_reported_to() {
        assert!(belongs_to("Demo App", &manager_session_id("Demo App")));
        assert!(belongs_to("Demo App", &tab_session_id("Demo App", "ab12cd34")));
        assert!(!belongs_to("Demo App", &tab_session_id("Other", "ab12cd34")));
        assert!(!belongs_to("Demo App", "0d1f-some-chat"));
    }

    #[tokio::test]
    async fn tabs_are_added_closed_and_reopened() {
        let dir = std::env::temp_dir().join(format!("forge-tabs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let home = dir.as_path();
        let a = add(home, "Demo", None).await.unwrap();
        let b = add(home, "Demo", Some("  Billing ")).await.unwrap();
        assert_eq!((a.title.as_str(), b.title.as_str()), ("Manager 2", "Billing"));
        assert_eq!(a.session_id, tab_session_id("Demo", &a.id));
        update(home, "Demo", &a.id, |t| t.closed_at = Some("now".into())).await.unwrap();
        let tabs = list(home, "Demo");
        assert!(!tabs[0].open() && tabs[1].open());
        update(home, "Demo", &a.id, |t| t.closed_at = None).await.unwrap();
        assert!(list(home, "Demo")[0].open());
        assert!(update(home, "Demo", "nope", |_| {}).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
