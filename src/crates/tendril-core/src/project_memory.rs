//! Long-term memory for a project: what agents and the operator have learned about it, kept across
//! every plan, mission and chat.
//!
//! Promptware memory (`promptware::memory`) is about how to do a *job* well, whatever the project.
//! This is about the *project*: its architecture, the conventions its code follows, decisions and
//! why they were made, the gotchas that cost a run, and the operator's preferences. Without it every
//! plan starts cold and relearns the same things.
//!
//! **Storage.** One markdown file per memory under `<TendrilHome>/Projects/<project>/Memory/`, with
//! YAML frontmatter, so a memory is readable and editable by hand and never touches the project's git
//! history.
//!
//! **Recall.** [`recall`] ranks memories against the work at hand: a BM25-style keyword score over
//! title, description, tags and body, plus a strong boost for a memory whose paths overlap the files
//! the work touches, plus a small one for preferences and conventions, which apply broadly. Every
//! job and plan chat on a project gets the index of all its memories and the most relevant ones in
//! full ([`render_for_prompt`]), so agents start with what matters and can read the rest.
//!
//! **Hygiene.** A memory whose paths no longer exist in any of the project's repos is reported
//! [`MemoryEntry::stale`]. Writes upsert by slug, so correcting a memory replaces it instead of
//! adding a contradicting one.

use crate::error::{Result, TendrilError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// What kind of thing a memory records. Unknown values read as `Note`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// How the system is put together: modules, data flow, where things live.
    Architecture,
    /// A rule the code follows: naming, patterns, testing, error handling.
    Convention,
    /// A choice that was made, and why.
    Decision,
    /// Something non-obvious that broke a run or will break the next one.
    Gotcha,
    /// What the operator wants: style, process, things to always or never do.
    Preference,
    #[default]
    #[serde(other)]
    Note,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::Convention => "convention",
            Self::Decision => "decision",
            Self::Gotcha => "gotcha",
            Self::Preference => "preference",
            Self::Note => "note",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "architecture" | "arch" => Self::Architecture,
            "convention" | "pattern" => Self::Convention,
            "decision" => Self::Decision,
            "gotcha" | "pitfall" | "warning" => Self::Gotcha,
            "preference" | "pref" => Self::Preference,
            _ => Self::Note,
        }
    }

    /// Applies broadly enough to be worth surfacing even without a keyword match.
    fn is_standing(&self) -> bool {
        matches!(self, Self::Preference | Self::Convention)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Frontmatter {
    title: String,
    #[serde(rename = "type", default)]
    kind: MemoryKind,
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    /// Who wrote it: `user`, `plan:00012`, `job:00018`, `chat:<id>`, `mission:00003`.
    #[serde(default)]
    source: String,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
}

/// One memory, as stored and as served.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    /// The file name without `.md`; the memory's id.
    pub slug: String,
    pub title: String,
    pub kind: MemoryKind,
    pub description: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub source: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub body: String,
    /// Every path it names is gone from the project's repos. Set by [`list_with_staleness`].
    #[serde(default)]
    pub stale: bool,
}

/// What a write supplies. A missing slug is derived from the title.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInput {
    #[serde(default)]
    pub slug: Option<String>,
    pub title: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub body: String,
}

/// `<TendrilHome>/Projects/<project>/Memory`: the same folder Settings' project memory table edits
/// and the Team Vault syncs, so all three see one memory.
pub fn memory_dir(tendril_home: &Path, project: &str) -> PathBuf {
    crate::config::get_project_memory_dir(tendril_home, project)
}

/// `Use React Query for server state` → `use-react-query-for-server-state`.
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(64).collect();
    if out.is_empty() { "memory".into() } else { out }
}

fn validate_slug(slug: &str) -> Result<String> {
    let slug = slugify(slug.trim().trim_end_matches(".md"));
    if slug.contains("..") || slug.contains('/') || slug.contains('\\') {
        return Err(TendrilError::Validation(format!("'{slug}' is not a valid memory id")));
    }
    Ok(slug)
}

/// A memory file, with or without frontmatter. One written by hand in the Settings editor (or synced
/// from a vault) has none: it reads as a note titled by its first heading, or its file name.
fn parse(slug: &str, text: &str) -> Option<MemoryEntry> {
    parse_frontmatter(slug, text).or_else(|| parse_plain(slug, text))
}

fn parse_plain(slug: &str, text: &str) -> Option<MemoryEntry> {
    let body = text.trim();
    if body.is_empty() {
        return None;
    }
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or_default();
    let (title, description) = match first.strip_prefix('#') {
        Some(heading) => (
            heading.trim_start_matches('#').trim().to_string(),
            lines.next().unwrap_or_default().trim_start_matches(['-', '*', '>', ' ']).to_string(),
        ),
        None => (slug.replace(['-', '_'], " "), first.chars().take(160).collect()),
    };
    let when = DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH);
    Some(MemoryEntry {
        slug: slug.to_string(),
        title,
        kind: MemoryKind::Note,
        description,
        paths: Vec::new(),
        tags: Vec::new(),
        source: "user".into(),
        created: when,
        updated: when,
        body: body.to_string(),
        stale: false,
    })
}

fn parse_frontmatter(slug: &str, text: &str) -> Option<MemoryEntry> {
    let rest = text.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let fm: Frontmatter = serde_yaml::from_str(&rest[..end]).ok()?;
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']).trim_end().to_string();
    Some(MemoryEntry {
        slug: slug.to_string(),
        title: fm.title,
        kind: fm.kind,
        description: fm.description,
        paths: fm.paths,
        tags: fm.tags,
        source: fm.source,
        created: fm.created,
        updated: fm.updated,
        body,
        stale: false,
    })
}

fn render(entry: &MemoryEntry) -> Result<String> {
    let fm = Frontmatter {
        title: entry.title.clone(),
        kind: entry.kind,
        description: entry.description.clone(),
        paths: entry.paths.clone(),
        tags: entry.tags.clone(),
        source: entry.source.clone(),
        created: entry.created,
        updated: entry.updated,
    };
    let yaml = serde_yaml::to_string(&fm).map_err(|e| TendrilError::Other(e.to_string()))?;
    Ok(format!("---\n{}---\n\n{}\n", yaml, entry.body.trim()))
}

/// Every memory of `project`, most recently updated first. Unreadable files are skipped.
pub fn list(tendril_home: &Path, project: &str) -> Vec<MemoryEntry> {
    let dir = memory_dir(tendril_home, project);
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<MemoryEntry> = read
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            let slug = path.file_stem()?.to_string_lossy().to_string();
            let mut entry = parse(&slug, &std::fs::read_to_string(&path).ok()?)?;
            // A plain file has no dates of its own: its modification time is the honest answer.
            if entry.updated.timestamp() == 0 {
                if let Ok(modified) = e.metadata().and_then(|m| m.modified()) {
                    entry.updated = modified.into();
                    entry.created = entry.updated;
                }
            }
            Some(entry)
        })
        .collect();
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

/// [`list`], with [`MemoryEntry::stale`] set against the project's repo roots.
pub fn list_with_staleness(tendril_home: &Path, project: &str, repo_roots: &[PathBuf]) -> Vec<MemoryEntry> {
    let mut entries = list(tendril_home, project);
    if repo_roots.is_empty() {
        return entries;
    }
    for e in entries.iter_mut() {
        e.stale = !e.paths.is_empty()
            && e.paths.iter().all(|p| {
                let rel = p.trim().trim_start_matches("./").trim_end_matches('/');
                !repo_roots.iter().any(|root| root.join(rel).exists())
            });
    }
    entries
}

pub fn get(tendril_home: &Path, project: &str, slug: &str) -> Result<MemoryEntry> {
    let slug = validate_slug(slug)?;
    let path = memory_dir(tendril_home, project).join(format!("{slug}.md"));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| TendrilError::Other(format!("No memory '{slug}' in project {project}")))?;
    parse(&slug, &text).ok_or_else(|| TendrilError::Validation(format!("Memory '{slug}' is not readable")))
}

/// Creates or replaces a memory. Replacing keeps its `created` time, so an edit is not a new memory.
pub fn write(tendril_home: &Path, project: &str, input: MemoryInput) -> Result<MemoryEntry> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(TendrilError::Validation("A memory needs a title".into()));
    }
    if input.body.trim().is_empty() && input.description.trim().is_empty() {
        return Err(TendrilError::Validation("A memory needs a description or a body".into()));
    }
    let slug = validate_slug(input.slug.as_deref().unwrap_or(title))?;
    let dir = memory_dir(tendril_home, project);
    std::fs::create_dir_all(&dir)?;
    let now = Utc::now();
    let created = get(tendril_home, project, &slug).map(|e| e.created).unwrap_or(now);
    let clean = |v: &[String]| -> Vec<String> {
        let mut seen = HashSet::new();
        v.iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
            .collect()
    };
    let entry = MemoryEntry {
        slug: slug.clone(),
        title: title.to_string(),
        kind: input.kind.as_deref().map(MemoryKind::from_str_loose).unwrap_or_default(),
        description: input.description.trim().to_string(),
        paths: clean(&input.paths),
        tags: clean(&input.tags),
        source: input.source.unwrap_or_else(|| "user".into()).trim().to_string(),
        created,
        updated: now,
        body: input.body.trim().to_string(),
        stale: false,
    };
    std::fs::write(dir.join(format!("{slug}.md")), render(&entry)?)?;
    Ok(entry)
}

pub fn delete(tendril_home: &Path, project: &str, slug: &str) -> Result<bool> {
    let slug = validate_slug(slug)?;
    let path = memory_dir(tendril_home, project).join(format!("{slug}.md"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Recall
// ---------------------------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "are", "was", "were", "will", "have",
    "has", "not", "but", "you", "your", "our", "its", "can", "all", "any", "use", "used", "using",
    "when", "then", "than", "them", "they", "what", "which", "who", "how", "why", "should", "would",
    "could", "must", "add", "make", "plan", "also", "only", "each", "every", "more", "new",
];

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// File and folder paths named in free text: `src/auth/session.rs`, `apps/admin/`, `file:///x/y.ts`.
static PATH_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?:file://)?/?(?:[A-Za-z0-9_.@-]+/)+[A-Za-z0-9_.@-]*").expect("path regex")
});

/// Web links, removed before looking for paths so `https://x.io/a/b` is not read as `x.io/a/b`.
static URL_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)\b(?:https?|ftp|ssh|git)://\S+").expect("url regex"));

pub fn paths_in(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let text = URL_RE.replace_all(text, " ");
    PATH_RE
        .find_iter(&text)
        .map(|m| m.as_str().trim_start_matches("file://").trim_end_matches(['.', ',']).to_string())
        .filter(|p| p.len() > 3 && !p.starts_with("http") && seen.insert(p.clone()))
        .collect()
}

/// Whether two repo paths are about the same place: one is the other, or contains it, or one ends
/// with the other (an absolute worktree path against a repo-relative memory path).
fn paths_overlap(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.trim().trim_start_matches("./").trim_end_matches('/').to_lowercase();
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    // `src/billing` is about `/wt/repo/src/billing/invoice.ts`: the shorter path appears in the longer
    // one as whole segments, at its start, its end, or anywhere between.
    let within = |long: &str, short: &str| {
        long == short
            || long.starts_with(&format!("{short}/"))
            || long.ends_with(&format!("/{short}"))
            || long.contains(&format!("/{short}/"))
    };
    within(&a, &b) || within(&b, &a)
}

/// The memories most relevant to work described by `query` and touching `paths`, best first, with
/// their scores. A memory with no keyword or path match is left out unless it is a standing
/// preference or convention.
pub fn recall(entries: &[MemoryEntry], query: &str, paths: &[String], limit: usize) -> Vec<(f64, MemoryEntry)> {
    let q: HashSet<String> = tokens(query).into_iter().collect();
    let docs: Vec<Vec<String>> = entries
        .iter()
        .map(|e| {
            // Title, description and tags count for more than the body: repeat them.
            let mut t = tokens(&e.title);
            t.extend(tokens(&e.title));
            t.extend(tokens(&e.title));
            t.extend(tokens(&e.description));
            t.extend(tokens(&e.description));
            for tag in &e.tags {
                t.extend(tokens(tag));
                t.extend(tokens(tag));
            }
            t.extend(tokens(&e.body));
            t
        })
        .collect();
    let n = docs.len().max(1) as f64;
    let avg_len = docs.iter().map(Vec::len).sum::<usize>() as f64 / n;
    let mut df: HashMap<&str, usize> = HashMap::new();
    for d in &docs {
        for w in d.iter().map(String::as_str).collect::<HashSet<_>>() {
            *df.entry(w).or_default() += 1;
        }
    }
    let (k1, b) = (1.2, 0.75);

    let mut scored: Vec<(f64, MemoryEntry)> = entries
        .iter()
        .zip(&docs)
        .filter_map(|(e, d)| {
            let mut tf: HashMap<&str, usize> = HashMap::new();
            for w in d {
                *tf.entry(w.as_str()).or_default() += 1;
            }
            let len = d.len() as f64;
            let mut keyword = 0.0;
            for w in &q {
                let Some(&f) = tf.get(w.as_str()) else { continue };
                let dfw = *df.get(w.as_str()).unwrap_or(&0) as f64;
                let idf = ((n - dfw + 0.5) / (dfw + 0.5) + 1.0).ln();
                let f = f as f64;
                keyword += idf * (f * (k1 + 1.0)) / (f + k1 * (1.0 - b + b * len / avg_len.max(1.0)));
            }
            let path_hits = e.paths.iter().filter(|mp| paths.iter().any(|p| paths_overlap(mp, p))).count();
            let path_score = if path_hits > 0 { 3.0 + path_hits as f64 } else { 0.0 };
            let standing = if e.kind.is_standing() { 0.75 } else { 0.0 };
            let stale_penalty = if e.stale { 0.5 } else { 1.0 };
            let score = (keyword + path_score + standing) * stale_penalty;
            (keyword > 0.0 || path_hits > 0 || e.kind.is_standing()).then_some((score, e.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// How many memories are listed by title, and how many inlined in full.
const INDEX_LIMIT: usize = 60;
const FULL_LIMIT: usize = 6;
const BODY_CHARS: usize = 1_500;

/// The `# Project Memory` section a job or plan chat gets: how to use and update memory, the index
/// of every memory, and the ones most relevant to this work in full. Empty memory still gets the
/// section, so agents know to start writing it.
pub fn render_for_prompt(tendril_home: &Path, project: &str, query: &str, paths: &[String]) -> String {
    let entries = list(tendril_home, project);
    let mut s = String::new();
    s.push_str("\n\n# Project Memory\n");
    s.push_str(&format!(
        "Long-term memory for project **{project}**: what earlier plans, missions, chats and the operator learned about it. It outranks your assumptions, but the code is the final word: if a memory is wrong or out of date, fix it.\n\n"
    ));
    if entries.is_empty() {
        s.push_str("There are no memories for this project yet.\n");
    } else {
        let relevant = recall(&entries, query, paths, FULL_LIMIT);
        if !relevant.is_empty() {
            s.push_str("## Most relevant to this work\n\n");
            for (_, e) in &relevant {
                s.push_str(&format!("### {} ({}, `{}`)\n", e.title, e.kind.as_str(), e.slug));
                if !e.description.is_empty() {
                    s.push_str(&format!("{}\n\n", e.description));
                }
                if !e.body.is_empty() {
                    let body: String = e.body.chars().take(BODY_CHARS).collect();
                    s.push_str(&body);
                    if e.body.chars().count() > BODY_CHARS {
                        s.push_str("\n…(truncated)");
                    }
                    s.push('\n');
                }
                if !e.paths.is_empty() {
                    s.push_str(&format!("Paths: {}\n", e.paths.join(", ")));
                }
                s.push('\n');
            }
        }
        let shown: HashSet<&str> = relevant.iter().map(|(_, e)| e.slug.as_str()).collect();
        let rest: Vec<&MemoryEntry> = entries.iter().filter(|e| !shown.contains(e.slug.as_str())).collect();
        if !rest.is_empty() {
            s.push_str("## Everything else (read with `tendril memory get`)\n");
            for e in rest.iter().take(INDEX_LIMIT) {
                s.push_str(&format!("- `{}` [{}] {}", e.slug, e.kind.as_str(), e.title));
                if !e.description.is_empty() {
                    s.push_str(&format!(" — {}", e.description));
                }
                s.push('\n');
            }
            if rest.len() > INDEX_LIMIT {
                s.push_str(&format!("…and {} more: `tendril memory list --project \"{project}\"`.\n", rest.len() - INDEX_LIMIT));
            }
            s.push('\n');
        }
    }
    s.push_str(&format!(
        r#"## Keeping it up to date
In your Reflection, save what a future plan on this project should know, as **project** memory (not promptware memory): an architecture fact, a convention the code follows, a decision and its reason, a gotcha that cost time, or something the operator asked for. One fact per memory, specific and verifiable, with the files it is about.

```bash
tendril memory write --project "{project}" --type convention --title "<short title>" \
  --description "<one line>" --path <repo/relative/path> --source "<plan:ID|job:ID>" <<'EOF'
<the detail: what, why, and how to apply it>
EOF
```

Before writing, check the list above: update an existing memory (`--slug <id>`) instead of adding a near-duplicate, and `tendril memory delete --project "{project}" <id>` one that turned out to be wrong. Do not save what the code or git history already says plainly, or anything only true for this one task.
"#
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(title: &str, kind: &str, body: &str, paths: &[&str]) -> MemoryInput {
        MemoryInput {
            title: title.into(),
            kind: Some(kind.into()),
            description: String::new(),
            paths: paths.iter().map(|p| p.to_string()).collect(),
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn writes_reads_upserts_and_deletes() {
        let home = tempfile::tempdir().unwrap();
        let a = write(home.path(), "My App", input("Use Zod for API validation", "convention", "All request bodies go through zod schemas.", &["src/api"])).unwrap();
        assert_eq!(a.slug, "use-zod-for-api-validation");
        assert_eq!(get(home.path(), "My App", &a.slug).unwrap().kind, MemoryKind::Convention);

        let mut changed = input("Use Zod for API validation", "convention", "Request and response bodies.", &["src/api"]);
        changed.slug = Some(a.slug.clone());
        let b = write(home.path(), "My App", changed).unwrap();
        assert_eq!(b.created, a.created, "an edit keeps its created time");
        assert_eq!(list(home.path(), "My App").len(), 1, "an upsert does not duplicate");

        assert!(delete(home.path(), "My App", &a.slug).unwrap());
        assert!(list(home.path(), "My App").is_empty());
        assert!(write(home.path(), "My App", input("", "note", "x", &[])).is_err());
        assert!(get(home.path(), "My App", "../../etc/passwd").is_err());
    }

    #[test]
    fn recall_prefers_keyword_and_path_matches() {
        let home = tempfile::tempdir().unwrap();
        let p = "P";
        write(home.path(), p, input("Kafka consumers must be idempotent", "gotcha", "Replays happen on rebalance; dedupe by event id.", &["src/arena/consumers"])).unwrap();
        write(home.path(), p, input("Postgres migrations use sqitch", "convention", "Never edit an applied migration.", &["db/migrations"])).unwrap();
        write(home.path(), p, input("Billing currency is cents", "decision", "Store integer cents everywhere.", &["src/billing"])).unwrap();
        let all = list(home.path(), p);

        let hits = recall(&all, "Add a new Kafka consumer for trades", &[], 3);
        assert_eq!(hits[0].1.slug, "kafka-consumers-must-be-idempotent");

        let by_path = recall(&all, "unrelated words", &["/wt/repo/src/billing/invoice.ts".into()], 3);
        assert_eq!(by_path[0].1.slug, "billing-currency-is-cents");

        // A convention is a standing rule: it is offered even with nothing matching.
        let none = recall(&all, "zzz", &[], 3);
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].1.kind, MemoryKind::Convention);
    }

    #[test]
    fn staleness_follows_the_repo() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("src/live")).unwrap();
        write(home.path(), "P", input("Live", "note", "x", &["src/live"])).unwrap();
        write(home.path(), "P", input("Gone", "note", "x", &["src/removed"])).unwrap();
        write(home.path(), "P", input("Pathless", "note", "x", &[])).unwrap();
        let entries = list_with_staleness(home.path(), "P", &[repo.path().to_path_buf()]);
        let stale: Vec<&str> = entries.iter().filter(|e| e.stale).map(|e| e.title.as_str()).collect();
        assert_eq!(stale, vec!["Gone"]);
    }

    #[test]
    fn a_hand_written_file_without_frontmatter_is_a_note() {
        let home = tempfile::tempdir().unwrap();
        let dir = memory_dir(home.path(), "P");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("deploys.md"), "# Deploys go through Argo\nNever kubectl apply by hand.\n").unwrap();
        let entries = list(home.path(), "P");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Deploys go through Argo");
        assert_eq!(entries[0].description, "Never kubectl apply by hand.");
        assert_eq!(entries[0].kind, MemoryKind::Note);
        assert!(entries[0].updated.timestamp() > 0, "dated by the file");
        assert_eq!(recall(&entries, "argo deploys", &[], 3).len(), 1);
    }

    #[test]
    fn finds_paths_in_plan_text() {
        let found = paths_in("Change [x](file:///repo/src/auth/session.rs) and src/api/routes/, see https://x.io/a/b.");
        assert!(found.contains(&"/repo/src/auth/session.rs".to_string()), "{found:?}");
        assert!(found.contains(&"src/api/routes".to_string()) || found.contains(&"src/api/routes/".to_string()), "{found:?}");
        assert!(!found.iter().any(|p| p.contains("x.io")), "{found:?}");
    }

    #[test]
    fn the_prompt_section_shows_relevant_memories_in_full_and_the_rest_by_title() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "P", input("Kafka consumers must be idempotent", "gotcha", "Dedupe by event id.", &[])).unwrap();
        write(home.path(), "P", input("Logo lives in the design repo", "note", "Not here.", &[])).unwrap();
        let s = render_for_prompt(home.path(), "P", "kafka consumer", &[]);
        assert!(s.contains("## Most relevant") && s.contains("Dedupe by event id."), "{s}");
        assert!(s.contains("`logo-lives-in-the-design-repo`"), "{s}");
        assert!(s.contains("tendril memory write --project \"P\""), "{s}");
    }
}
