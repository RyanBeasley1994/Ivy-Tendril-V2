//! What a project has running somewhere: the preview deploy, the local dev server an agent started,
//! the staging admin and the test account that logs into it.
//!
//! Agents find these out mid-run and say so once in a chat, where they scroll away. Here they are
//! recorded (`tendril link set`) and shown on the project page, and every job and chat on the project
//! is handed the list ([`render_for_prompt`]) so the next agent reuses the URL and the login instead
//! of asking for them again.
//!
//! **Storage.** One JSON array at `<TendrilHome>/Projects/<project>/Links.json`, outside the project's
//! git history. Credentials here are meant for **test** accounts; they are stored in plain text.
//!
//! **Upsert.** A write with a slug replaces that link; without one, a link already pointing at the
//! same URL is replaced, so an agent re-announcing the dev server does not add a second row.

use crate::config::get_project_root_dir;
use crate::error::{Result, TendrilError};
use crate::fs_lock::{with_file_lock, write_atomic};
use crate::project_memory::slugify;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLink {
    pub slug: String,
    pub label: String,
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Anything else needed to use it: which account is admin, that it resets nightly, an API key's
    /// header name.
    #[serde(default)]
    pub notes: String,
    /// Who recorded it: plan:ID, job:ID, mission:ID, chat:ID or user.
    #[serde(default)]
    pub source: String,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LinkInput {
    /// Replace this link instead of matching by URL.
    pub slug: Option<String>,
    pub label: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub notes: String,
    pub source: Option<String>,
}

pub fn links_path(tendril_home: &Path, project: &str) -> PathBuf {
    get_project_root_dir(tendril_home, project).join("Links.json")
}

fn read(path: &Path) -> Vec<ProjectLink> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(path: &Path, links: &[ProjectLink]) -> Result<()> {
    let json = serde_json::to_vec_pretty(links).map_err(|e| TendrilError::Other(e.to_string()))?;
    write_atomic(path, &json)
}

/// Every link, most recently updated first.
pub fn list(tendril_home: &Path, project: &str) -> Vec<ProjectLink> {
    let mut links = read(&links_path(tendril_home, project));
    links.sort_by(|a, b| b.updated.cmp(&a.updated));
    links
}

pub fn set(tendril_home: &Path, project: &str, input: LinkInput) -> Result<ProjectLink> {
    let url = input.url.trim().to_string();
    if url.is_empty() {
        return Err(TendrilError::Validation("A link needs a URL".into()));
    }
    let label = match input.label.trim() {
        "" => url.clone(),
        l => l.to_string(),
    };
    let path = links_path(tendril_home, project);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    with_file_lock(&path, || {
        let mut links = read(&path);
        let wanted = input.slug.as_deref().map(slugify);
        let existing = match &wanted {
            Some(slug) => links.iter().position(|l| &l.slug == slug),
            None => links.iter().position(|l| l.url == url),
        };
        let slug = match (&wanted, existing) {
            (Some(slug), _) => slug.clone(),
            (None, Some(i)) => links[i].slug.clone(),
            (None, None) => unique_slug(&links, &slugify(&label)),
        };
        let link = ProjectLink {
            slug,
            label,
            url,
            username: input.username.trim().to_string(),
            password: input.password,
            notes: input.notes.trim().to_string(),
            source: input.source.unwrap_or_else(|| "user".into()),
            updated: Utc::now(),
        };
        match existing {
            Some(i) => links[i] = link.clone(),
            None => links.push(link.clone()),
        }
        save(&path, &links)?;
        Ok(link)
    })
}

/// Whether a link was there to remove.
pub fn remove(tendril_home: &Path, project: &str, slug: &str) -> Result<bool> {
    let path = links_path(tendril_home, project);
    if !path.exists() {
        return Ok(false);
    }
    with_file_lock(&path, || {
        let mut links = read(&path);
        let before = links.len();
        links.retain(|l| l.slug != slug);
        if links.len() == before {
            return Ok(false);
        }
        save(&path, &links)?;
        Ok(true)
    })
}

fn unique_slug(links: &[ProjectLink], base: &str) -> String {
    let taken = |s: &str| links.iter().any(|l| l.slug == s);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|s| !taken(s)).unwrap_or_default()
}

/// The prompt section every job and chat on the project gets: what is already hosted, and how to
/// record something new.
pub fn render_for_prompt(tendril_home: &Path, project: &str) -> String {
    let links = list(tendril_home, project);
    let mut s = String::from("\n\n# Hosted\n");
    if links.is_empty() {
        s.push_str("Nothing is recorded as hosted for this project yet.\n");
    } else {
        s.push_str("What this project has running, with its test logins. Use these rather than asking for them.\n\n");
        for l in &links {
            s.push_str(&format!("- **{}**: {}", l.label, l.url));
            if !l.username.is_empty() || !l.password.is_empty() {
                s.push_str(&format!(" (login `{}` / `{}`)", l.username, l.password));
            }
            if !l.notes.is_empty() {
                s.push_str(&format!(" — {}", l.notes));
            }
            s.push_str(&format!(" [`{}`]\n", l.slug));
        }
    }
    s.push_str(&format!(
        r#"
When you host, deploy or start something the operator should be able to open (a dev server, a preview deploy, a staging site, an admin panel, a seeded test account), record it so it shows on the project page instead of getting lost in the chat. Use a URL they can reach, never a secret that is not a throwaway test credential:

```bash
tendril link set --project "{project}" --label "<what it is>" --url <url> \
  [--username <test user>] [--password <test password>] [--notes "<anything else needed>"]
```

Setting the same URL again updates it; `tendril link remove --project "{project}" <slug>` once it is gone.
"#
    ));
    s
}

/// The short form a manager sees at the top of every turn: what is recorded, and the one command that
/// keeps it current. A manager hears about hosted things first (it starts the dev servers and tunnels,
/// or a worker reports them), so it is the one that has to write them down.
pub fn render_for_manager(tendril_home: &Path, project: &str) -> String {
    let links = list(tendril_home, project);
    let mut s = String::from("Hosted, as shown on the project page:\n");
    if links.is_empty() {
        s.push_str("- nothing recorded\n");
    }
    for l in &links {
        s.push_str(&format!("- [{}] {}: {}", l.slug, l.label, l.url));
        if !l.username.is_empty() || !l.password.is_empty() {
            s.push_str(&format!(" (login {} / {})", l.username, l.password));
        }
        if !l.notes.is_empty() {
            s.push_str(&format!(" — {}", l.notes));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "Whenever you or a worker start, find or restart something the operator can open (dev server, tunnel, preview, staging, admin), record it with its test login in the same turn: `tendril link set --project \"{project}\" --label <what> --url <url> [--username u] [--password p] [--notes ...]` (add `--slug <id>` when its URL changed); `tendril link remove --project \"{project}\" <id>` when it is gone. Keep this list true.\n"
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(label: &str, url: &str) -> LinkInput {
        LinkInput { label: label.into(), url: url.into(), ..Default::default() }
    }

    #[test]
    fn setting_the_same_url_updates_instead_of_adding() {
        let home = tempfile::tempdir().unwrap();
        set(home.path(), "P", input("Dev server", "http://localhost:3000")).unwrap();
        let again = set(
            home.path(),
            "P",
            LinkInput { password: "hunter2".into(), ..input("Dev", "http://localhost:3000") },
        )
        .unwrap();
        let links = list(home.path(), "P");
        assert_eq!(links.len(), 1);
        assert_eq!(again.slug, "dev-server");
        assert_eq!(links[0].label, "Dev");
        assert_eq!(links[0].password, "hunter2");
    }

    #[test]
    fn labels_that_collide_get_distinct_slugs_and_remove_by_slug() {
        let home = tempfile::tempdir().unwrap();
        let a = set(home.path(), "P", input("Preview", "https://a.example.com")).unwrap();
        let b = set(home.path(), "P", input("Preview", "https://b.example.com")).unwrap();
        assert_eq!((a.slug.as_str(), b.slug.as_str()), ("preview", "preview-2"));
        assert!(remove(home.path(), "P", "preview").unwrap());
        assert!(!remove(home.path(), "P", "preview").unwrap());
        assert_eq!(list(home.path(), "P").len(), 1);
    }

    #[test]
    fn a_link_needs_a_url() {
        let home = tempfile::tempdir().unwrap();
        assert!(set(home.path(), "P", input("Nothing", "  ")).is_err());
    }

    #[test]
    fn the_prompt_carries_logins_and_the_command() {
        let home = tempfile::tempdir().unwrap();
        set(
            home.path(),
            "P",
            LinkInput { username: "admin".into(), password: "pw".into(), ..input("Admin", "https://x.dev/admin") },
        )
        .unwrap();
        let s = render_for_prompt(home.path(), "P");
        assert!(s.contains("**Admin**: https://x.dev/admin (login `admin` / `pw`)"), "{s}");
        assert!(s.contains("tendril link set --project \"P\""), "{s}");
        let m = render_for_manager(home.path(), "P");
        assert!(m.contains("- [admin] Admin: https://x.dev/admin (login admin / pw)"), "{m}");
        assert!(m.contains("--slug <id>"), "{m}");
    }
}
