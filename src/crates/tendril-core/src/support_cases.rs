//! Triage for support tickets: what the support manager may tell a partner, and on what evidence.
//!
//! A new ticket goes to the one project manager the operator picked for support
//! ([`SupportSettings::manager_project`]). It investigates read-only, records what it found
//! ([`Investigation`]), and only then may it reply. The rules are enforced here, not left to the
//! briefing:
//!
//! * **Evidence first.** Every read-only lookup the manager runs (`tendril support db` / `api`) is
//!   logged on the ticket's case, and a reply is refused until an investigation is recorded that
//!   cites lookups or code it read.
//! * **What may be said.** A bug may be acknowledged ("we're looking into it"). A question may be
//!   answered only at 90% confidence or more. "Fixed in the next update" needs a micro fix (a PR of
//!   at most [`MICRO_FIX_LINES`] changed lines, merged or green) and 100% confidence. A request gets
//!   no reply at all. Below the bar the manager writes an internal note for the operator instead.
//! * **Approval.** While [`ReplyMode::Approve`] is set (the default) a reply is held as a draft the
//!   operator approves on the ticket; nothing reaches the partner until they do.
//! * **Nothing internal leaks.** A reply carrying ids, connection strings, file paths, stack traces or
//!   someone else's email address is refused.
//!
//! `<TendrilHome>/support-settings.json` holds the settings and data sources (owner-only: source URLs
//! carry credentials). `<TendrilHome>/support-cases/<ticket>.json` holds each ticket's case.

use crate::error::{Result, TendrilError};
use crate::fs_lock::{with_file_lock, write_atomic};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The largest fix, in changed lines, that may be promised to a partner as resolved.
pub const MICRO_FIX_LINES: u64 = 60;
/// The confidence a question's answer needs before it may be sent.
pub const ANSWER_CONFIDENCE: u8 = 90;

/* ------------------------------------------------------------------ settings */

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyMode {
    /// Replies wait for the operator's approval on the ticket.
    #[default]
    Approve,
    /// Replies that pass the rules are sent at once.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Mongo,
    Http,
}

/// Somewhere the manager may read real data from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    pub name: String,
    pub kind: SourceKind,
    /// A MongoDB connection string (use a user with the `read` role only), or an API base URL.
    pub url: String,
    /// For an API: one header sent with every request, e.g. `Authorization: Bearer ...`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// What is in it, for the manager: "accounts, challenges, payouts", "the admin API".
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SupportSettings {
    /// The project whose manager works every ticket.
    pub manager_project: Option<String>,
    /// Hand new tickets to that manager as they arrive.
    pub auto_triage: bool,
    /// When auto triage was turned on: tickets older than this are left alone.
    pub triage_since: Option<DateTime<Utc>>,
    pub reply_mode: ReplyMode,
    pub sources: Vec<DataSource>,
    /// The manager may read the cluster through the daemon machine's own kubeconfig
    /// (`support_kube`): resources, logs, and databases and services reached by port-forward.
    pub kubernetes: bool,
    /// The kubeconfig context to use; the current one when empty.
    pub kube_context: Option<String>,
}

/// A source as the UI sees it: the URL with any password masked, the header's value hidden.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub has_header: bool,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub manager_project: Option<String>,
    pub auto_triage: bool,
    pub reply_mode: ReplyMode,
    pub sources: Vec<SourceView>,
    pub kubernetes: bool,
    pub kube_context: Option<String>,
}

static URL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(://[^:/@\s]+):[^@/\s]+@").expect("url password regex"));

pub fn mask_url(url: &str) -> String {
    URL_PASSWORD.replace(url, "$1:••••@").to_string()
}

fn settings_path(home: &Path) -> PathBuf {
    home.join("support-settings.json")
}

pub fn load_settings(home: &Path) -> SupportSettings {
    std::fs::read_to_string(settings_path(home))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn view(settings: &SupportSettings) -> SettingsView {
    SettingsView {
        manager_project: settings.manager_project.clone(),
        auto_triage: settings.auto_triage,
        reply_mode: settings.reply_mode,
        sources: settings
            .sources
            .iter()
            .map(|s| SourceView {
                name: s.name.clone(),
                kind: s.kind,
                url: mask_url(&s.url),
                has_header: s.header.as_deref().is_some_and(|h| !h.trim().is_empty()),
                notes: s.notes.clone(),
            })
            .collect(),
        kubernetes: settings.kubernetes,
        kube_context: settings.kube_context.clone(),
    }
}

/// A source as the UI sends it back: a URL or header left as shown (masked) keeps the saved secret.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInput {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInput {
    pub manager_project: Option<String>,
    pub auto_triage: bool,
    pub reply_mode: ReplyMode,
    pub sources: Vec<SourceInput>,
    #[serde(default)]
    pub kubernetes: bool,
    #[serde(default)]
    pub kube_context: Option<String>,
}

pub fn save_settings(home: &Path, input: SettingsInput) -> Result<SettingsView> {
    let old = load_settings(home);
    let mut sources = Vec::new();
    for s in input.sources {
        let name = s.name.trim().to_string();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(TendrilError::Validation("Each data source needs a one-word name".into()));
        }
        if sources.iter().any(|o: &DataSource| o.name.eq_ignore_ascii_case(&name)) {
            return Err(TendrilError::Validation(format!("Two data sources are called '{name}'")));
        }
        let previous = old.sources.iter().find(|o| o.name.eq_ignore_ascii_case(&name));
        let url = match previous {
            Some(p) if s.url.trim() == mask_url(&p.url) => p.url.clone(),
            _ => s.url.trim().to_string(),
        };
        if url.is_empty() {
            return Err(TendrilError::Validation(format!("Data source '{name}' needs a URL")));
        }
        let header = match s.header.as_deref().map(str::trim) {
            // Blank means "keep what is saved"; the UI never sees the value to send it back.
            None | Some("") => previous.and_then(|p| p.header.clone()),
            Some("-") => None,
            Some(h) => Some(h.to_string()),
        };
        sources.push(DataSource { name, kind: s.kind, url, header, notes: s.notes.trim().to_string() });
    }
    let manager_project = input.manager_project.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let triage_since = match (input.auto_triage, old.auto_triage) {
        (true, true) => old.triage_since.or(Some(Utc::now())),
        (true, false) => Some(Utc::now()),
        _ => None,
    };
    let settings = SupportSettings {
        manager_project,
        auto_triage: input.auto_triage,
        triage_since,
        reply_mode: input.reply_mode,
        sources,
        kubernetes: input.kubernetes,
        kube_context: input.kube_context.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
    };
    let path = settings_path(home);
    std::fs::create_dir_all(home)?;
    let json = serde_json::to_vec_pretty(&settings).map_err(|e| TendrilError::Other(e.to_string()))?;
    write_atomic(&path, &json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(view(&settings))
}

pub fn source(home: &Path, name: &str) -> Result<DataSource> {
    let settings = load_settings(home);
    settings
        .sources
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
        .cloned()
        .ok_or_else(|| {
            let names: Vec<&str> = settings.sources.iter().map(|s| s.name.as_str()).collect();
            TendrilError::Validation(format!(
                "No data source called '{name}'. Configured: {}",
                if names.is_empty() { "none (add them on the Support page)".to_string() } else { names.join(", ") }
            ))
        })
}

/* ------------------------------------------------------------------ cases */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Bug,
    Question,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Critical,
    High,
    Normal,
    Low,
}

/// One read-only lookup the manager ran for the ticket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lookup {
    pub at: DateTime<Utc>,
    pub source: String,
    pub request: String,
    /// What came back, in a few words: "3 documents", "200, 2.1 kB", or the error.
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Investigation {
    pub at: DateTime<Utc>,
    pub category: Category,
    pub severity: Severity,
    /// 0-100: how sure the manager is of its findings (for a question, of its answer).
    pub confidence: u8,
    pub findings: String,
    /// Code read, records checked: `file:line`, collection and id, endpoint.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Another ticket about the same problem, which this one follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyKind {
    /// "We're looking into it", for a bug.
    Ack,
    /// The answer to a question.
    Answer,
    /// "Resolved in the next update", for a verified micro fix.
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DraftState {
    Pending,
    Sent,
    Discarded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    pub kind: ReplyKind,
    pub content: String,
    /// Resolve the ticket once this is sent.
    #[serde(default)]
    pub close: bool,
    /// The fix a `Resolved` reply rests on, and the test that shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_pr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    pub created_at: DateTime<Utc>,
    pub state: DraftState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Case {
    pub ticket_id: String,
    /// The ticket was handed to the support manager automatically.
    pub triaged_at: Option<DateTime<Utc>>,
    pub subject: String,
    pub lookups: Vec<Lookup>,
    pub investigation: Option<Investigation>,
    pub drafts: Vec<Draft>,
    /// The operator was alerted that this ticket is critical.
    pub alerted: bool,
    /// The last partner message passed on to the manager's conversation.
    pub last_forwarded: Option<String>,
}

impl Case {
    pub fn pending_drafts(&self) -> usize {
        self.drafts.iter().filter(|d| d.state == DraftState::Pending).count()
    }
}

fn cases_dir(home: &Path) -> PathBuf {
    home.join("support-cases")
}

fn safe(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect()
}

fn case_path(home: &Path, ticket: &str) -> PathBuf {
    cases_dir(home).join(format!("{}.json", safe(ticket)))
}

pub fn load_case(home: &Path, ticket: &str) -> Case {
    std::fs::read_to_string(case_path(home, ticket))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Case { ticket_id: ticket.to_string(), ..Default::default() })
}

/// Changes a ticket's case under its file lock and returns it as saved.
pub fn update_case<T>(home: &Path, ticket: &str, f: impl FnOnce(&mut Case) -> T) -> Result<(Case, T)> {
    if safe(ticket).is_empty() {
        return Err(TendrilError::Validation("Not a ticket id".into()));
    }
    std::fs::create_dir_all(cases_dir(home))?;
    let path = case_path(home, ticket);
    with_file_lock(&path, || {
        let mut case = load_case(home, ticket);
        case.ticket_id = ticket.to_string();
        let out = f(&mut case);
        let json = serde_json::to_vec_pretty(&case).map_err(|e| TendrilError::Other(e.to_string()))?;
        write_atomic(&path, &json)?;
        Ok((case, out))
    })
}

pub fn all_cases(home: &Path) -> Vec<Case> {
    std::fs::read_dir(cases_dir(home))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| serde_json::from_str(&t).ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn log_lookup(home: &Path, ticket: &str, source: &str, request: &str, result: &str) -> Result<()> {
    update_case(home, ticket, |c| {
        c.lookups.push(Lookup {
            at: Utc::now(),
            source: source.to_string(),
            request: request.chars().take(600).collect(),
            result: result.chars().take(200).collect(),
        });
    })
    .map(|_| ())
}

pub fn record_investigation(home: &Path, ticket: &str, investigation: Investigation) -> Result<Case> {
    if investigation.confidence > 100 {
        return Err(TendrilError::Validation("Confidence is 0 to 100".into()));
    }
    if investigation.findings.trim().len() < 20 {
        return Err(TendrilError::Validation("Write what you found: the findings are what the operator reads".into()));
    }
    if investigation.duplicate_of.as_deref() == Some(ticket) {
        return Err(TendrilError::Validation("A ticket cannot duplicate itself".into()));
    }
    update_case(home, ticket, |c| {
        // A recategorised ticket is re-alerted only if it became critical.
        if investigation.severity != Severity::Critical {
            c.alerted = c.alerted && c.investigation.as_ref().is_some_and(|i| i.severity == Severity::Critical);
        }
        c.investigation = Some(investigation);
    })
    .map(|(c, _)| c)
}

/* ------------------------------------------------------------------ the gate */

static OBJECT_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[0-9a-f]{24}\b").expect("object id regex"));
static UUID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").expect("uuid regex"));
static CONNECTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(mongodb(\+srv)?|postgres(ql)?|mysql|redis)://").expect("connection regex"));
static FILE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\b[\w./-]+\.(rs|ts|tsx|js|jsx|py|go|cs|json|ya?ml)(:\d+)?\b|/(users|home|var|etc|srv|app)/)").expect("path regex")
});
static STACK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)(^\s+at [\w.<>]+ \(|Traceback \(most recent|panicked at|Exception:|```)").expect("stack regex"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b[\w.+-]+@[\w-]+\.[\w.-]+\b").expect("email regex"));
static SECRETISH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(api[_-]?key|secret|password|token|bearer)\b\s*[:=]").expect("secret regex"));

/// What in `content` must not reach a partner, if anything. `allowed_emails` are the ticket's own
/// people, whose addresses may be repeated back to them.
pub fn privacy_problems(content: &str, allowed_emails: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    if OBJECT_ID.is_match(content) || UUID.is_match(content) {
        problems.push("it contains a database or record id".to_string());
    }
    if CONNECTION.is_match(content) {
        problems.push("it contains a connection string".to_string());
    }
    if FILE_PATH.is_match(content) {
        problems.push("it names source files or server paths".to_string());
    }
    if STACK.is_match(content) {
        problems.push("it contains code, a stack trace or an error dump".to_string());
    }
    if SECRETISH.is_match(content) {
        problems.push("it looks like it carries a credential".to_string());
    }
    let foreign: Vec<String> = EMAIL
        .find_iter(content)
        .map(|m| m.as_str().to_ascii_lowercase())
        .filter(|e| !allowed_emails.iter().any(|a| a.eq_ignore_ascii_case(e)))
        .collect();
    if !foreign.is_empty() {
        problems.push(format!("it mentions an email address that is not this ticket's ({})", foreign.join(", ")));
    }
    problems
}

/// What a `Resolved` reply's pull request looks like on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixPr {
    pub state: String,
    pub changed_lines: u64,
    /// Every check finished and passed (or there are none).
    pub checks_green: bool,
}

pub type FixPrLookup<'a> = &'a dyn Fn(&str) -> Result<FixPr>;

/// A reply the manager wants to send.
#[derive(Debug, Clone)]
pub struct ReplyRequest {
    pub kind: ReplyKind,
    pub content: String,
    pub close: bool,
    pub fix_pr: Option<String>,
    pub test: Option<String>,
}

/// Why `request` may not go to the partner on `case`, or `Ok` when it may.
pub fn check_reply(case: &Case, request: &ReplyRequest, allowed_emails: &[String], fix_pr: FixPrLookup) -> Result<()> {
    let refuse = |why: String| Err(TendrilError::Validation(why));
    let Some(inv) = &case.investigation else {
        return refuse(
            "Investigate first: run your read-only lookups, then record what you found with `tendril support investigate`. No reply goes out before that.".into(),
        );
    };
    let note = "Write your findings as an internal note for the operator instead (`tendril support note`).";
    if inv.category == Category::Request {
        return refuse(format!("This ticket is a request, and requests get no reply. {note}"));
    }
    if case.lookups.is_empty() && inv.evidence.is_empty() {
        return refuse(format!(
            "Your investigation cites no evidence: no lookups were run on this ticket and none were listed with --evidence. {note}"
        ));
    }
    if request.content.trim().len() < 10 {
        return refuse("Write the reply in full.".into());
    }
    match request.kind {
        ReplyKind::Ack if inv.category != Category::Bug => {
            return refuse("Only a bug gets a \"we're looking into it\" reply.".into());
        }
        ReplyKind::Answer if inv.category != Category::Question => {
            return refuse("Only a question gets an answer. A bug gets --kind ack, then --kind resolved once fixed.".into());
        }
        ReplyKind::Answer if inv.confidence < ANSWER_CONFIDENCE => {
            return refuse(format!(
                "You are {}% sure, and an answer needs {ANSWER_CONFIDENCE}%. {note}",
                inv.confidence
            ));
        }
        ReplyKind::Resolved => {
            if inv.category != Category::Bug {
                return refuse("Only a bug is resolved by a fix.".into());
            }
            if inv.confidence < 100 {
                return refuse(format!(
                    "\"Resolved\" needs you to be certain the fix works, and you recorded {}%. Leave it to the operator. {note}",
                    inv.confidence
                ));
            }
            let Some(url) = request.fix_pr.as_deref().filter(|u| u.contains("/pull/")) else {
                return refuse("Name the fix's pull request with --fix-pr.".into());
            };
            if request.test.as_deref().map(str::trim).unwrap_or_default().len() < 10 {
                return refuse("Say which test reproduces the bug and now passes, with --test.".into());
            }
            let pr = fix_pr(url)?;
            if pr.changed_lines > MICRO_FIX_LINES {
                return refuse(format!(
                    "The fix changes {} lines, and only a micro fix (up to {MICRO_FIX_LINES}) may be promised to a partner. {note}",
                    pr.changed_lines
                ));
            }
            let merged = pr.state.eq_ignore_ascii_case("MERGED");
            if !merged && !(pr.state.eq_ignore_ascii_case("OPEN") && pr.checks_green) {
                return refuse(format!("The fix's pull request is {} and not green yet.", pr.state.to_lowercase()));
            }
        }
        _ => {}
    }
    let problems = privacy_problems(&request.content, allowed_emails);
    if !problems.is_empty() {
        return refuse(format!(
            "That reply can't go to a partner: {}. Say it in plain words, without internal details.",
            problems.join("; ")
        ));
    }
    Ok(())
}

/// Queues `request` as a pending draft on the ticket.
pub fn add_draft(home: &Path, ticket: &str, request: &ReplyRequest) -> Result<Draft> {
    let draft = Draft {
        id: uuid::Uuid::new_v4().to_string(),
        kind: request.kind,
        content: request.content.trim().to_string(),
        close: request.close,
        fix_pr: request.fix_pr.clone(),
        test: request.test.clone(),
        created_at: Utc::now(),
        state: DraftState::Pending,
        decided_at: None,
    };
    let saved = draft.clone();
    update_case(home, ticket, |c| {
        // A newer draft of the same kind replaces one still waiting.
        for d in c.drafts.iter_mut().filter(|d| d.state == DraftState::Pending && d.kind == saved.kind) {
            d.state = DraftState::Discarded;
            d.decided_at = Some(Utc::now());
        }
        c.drafts.push(saved);
    })?;
    Ok(draft)
}

/// Marks a draft sent or discarded, returning it as it was decided.
pub fn decide_draft(home: &Path, ticket: &str, draft_id: &str, state: DraftState, content: Option<&str>) -> Result<Draft> {
    let (_, found) = update_case(home, ticket, |c| {
        let d = c.drafts.iter_mut().find(|d| d.id == draft_id && d.state == DraftState::Pending)?;
        if let Some(text) = content.map(str::trim).filter(|t| !t.is_empty()) {
            d.content = text.to_string();
        }
        d.state = state;
        d.decided_at = Some(Utc::now());
        Some(d.clone())
    })?;
    found.ok_or_else(|| TendrilError::Validation("That draft is not waiting any more".into()))
}

/// The other tickets following `ticket` as duplicates.
pub fn duplicates_of(home: &Path, ticket: &str) -> Vec<Case> {
    all_cases(home)
        .into_iter()
        .filter(|c| c.investigation.as_ref().and_then(|i| i.duplicate_of.as_deref()) == Some(ticket))
        .collect()
}

/// The open bugs already being worked, for spotting a duplicate: `(ticket, subject, findings)`.
pub fn open_bugs(home: &Path, except: &str) -> Vec<(String, String, String)> {
    all_cases(home)
        .into_iter()
        .filter(|c| c.ticket_id != except)
        .filter_map(|c| {
            let inv = c.investigation.as_ref()?;
            let resolved = c.drafts.iter().any(|d| d.kind == ReplyKind::Resolved && d.state == DraftState::Sent);
            (inv.category == Category::Bug && inv.duplicate_of.is_none() && !resolved)
                .then(|| (c.ticket_id.clone(), c.subject.clone(), inv.findings.chars().take(160).collect()))
        })
        .collect()
}

/* ------------------------------------------------------------------ history */

/// One finished ticket, remembered for the next one like it: what it was, what was found, how it was
/// fixed and what the partner was told. Kept apart from project memory in
/// `<TendrilHome>/support-history.jsonl`, one entry per ticket (a later write replaces an earlier one).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub ticket_id: String,
    pub at: DateTime<Utc>,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<Category>,
    pub findings: String,
    /// How it was fixed or settled, in a sentence or two.
    #[serde(default)]
    pub resolution: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_pr: Option<String>,
    /// What the partner was told, in order.
    #[serde(default)]
    pub replies: Vec<String>,
}

fn history_path(home: &Path) -> PathBuf {
    home.join("support-history.jsonl")
}

pub fn load_history(home: &Path) -> Vec<HistoryEntry> {
    let mut entries: Vec<HistoryEntry> = std::fs::read_to_string(history_path(home))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    // The last write for a ticket wins.
    let mut seen = std::collections::HashSet::new();
    entries.reverse();
    entries.retain(|e| seen.insert(e.ticket_id.clone()));
    entries.reverse();
    entries
}

pub fn remember(home: &Path, entry: &HistoryEntry) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(home)?;
    let path = history_path(home);
    let line = serde_json::to_string(entry).map_err(|e| TendrilError::Other(e.to_string()))?;
    with_file_lock(&path, || {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "{line}")?;
        Ok(())
    })
}

/// The history entry a case makes once it is finished, merged over any `resolution` already written
/// for it (the manager's own `tendril support learn`).
pub fn entry_from_case(home: &Path, case: &Case, partner: Option<String>, resolution: Option<&str>) -> HistoryEntry {
    let previous = load_history(home).into_iter().find(|e| e.ticket_id == case.ticket_id);
    let sent: Vec<&Draft> = case.drafts.iter().filter(|d| d.state == DraftState::Sent).collect();
    HistoryEntry {
        ticket_id: case.ticket_id.clone(),
        at: Utc::now(),
        subject: case.subject.clone(),
        partner: partner.or_else(|| previous.as_ref().and_then(|p| p.partner.clone())),
        category: case.investigation.as_ref().map(|i| i.category),
        findings: case.investigation.as_ref().map(|i| i.findings.clone()).unwrap_or_default(),
        resolution: resolution
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string)
            .or_else(|| previous.as_ref().map(|p| p.resolution.clone()).filter(|r| !r.is_empty()))
            .unwrap_or_default(),
        fix_pr: sent
            .iter()
            .rev()
            .find_map(|d| d.fix_pr.clone())
            .or_else(|| previous.as_ref().and_then(|p| p.fix_pr.clone())),
        replies: sent.iter().map(|d| d.content.clone()).collect(),
    }
}

fn words(text: &str) -> std::collections::HashSet<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "that", "this", "with", "have", "but", "not", "are", "was", "you", "your", "our", "can",
        "from", "they", "when", "what", "there", "been", "has", "any", "please", "thanks", "hi", "hello", "just",
    ];
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(str::to_string)
        .collect()
}

/// The past tickets most like `text`, best first: word overlap over subject, findings and resolution.
pub fn similar(home: &Path, text: &str, except: Option<&str>, limit: usize) -> Vec<HistoryEntry> {
    let want = words(text);
    if want.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(f64, HistoryEntry)> = load_history(home)
        .into_iter()
        .filter(|e| Some(e.ticket_id.as_str()) != except)
        .filter_map(|e| {
            let have = words(&format!("{} {} {} {}", e.subject, e.subject, e.findings, e.resolution));
            let shared = want.intersection(&have).count() as f64;
            let score = shared / ((want.len() as f64).sqrt() * (have.len().max(1) as f64).sqrt());
            (shared >= 2.0).then_some((score, e))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, e)| e).collect()
}

/// `entries` as the manager reads them in its brief.
pub fn render_history(entries: &[HistoryEntry]) -> String {
    let mut s = String::new();
    for e in entries {
        let kind = match e.category {
            Some(Category::Bug) => "bug",
            Some(Category::Question) => "question",
            Some(Category::Request) => "request",
            None => "ticket",
        };
        s.push_str(&format!("- **{}** ({kind}, {}, ticket `{}`)\n", e.subject, e.at.format("%Y-%m-%d"), e.ticket_id));
        if !e.findings.is_empty() {
            s.push_str(&format!("  Found: {}\n", e.findings.trim()));
        }
        if !e.resolution.is_empty() {
            s.push_str(&format!("  Settled: {}\n", e.resolution.trim()));
        }
        if let Some(pr) = &e.fix_pr {
            s.push_str(&format!("  Fix: {pr}\n"));
        }
        for r in &e.replies {
            s.push_str(&format!("  Told the partner: \"{}\"\n", r.trim().replace('\n', " ")));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_tickets_are_found_and_the_latest_entry_wins() {
        let home = tempfile::tempdir().unwrap();
        let entry = |id: &str, subject: &str, resolution: &str| HistoryEntry {
            ticket_id: id.into(),
            at: Utc::now(),
            subject: subject.into(),
            partner: None,
            category: Some(Category::Bug),
            findings: String::new(),
            resolution: resolution.into(),
            fix_pr: None,
            replies: vec![],
        };
        remember(home.path(), &entry("a", "Payout stuck pending after KYC", "first")).unwrap();
        remember(home.path(), &entry("b", "Dashboard theme colours wrong", "x")).unwrap();
        remember(home.path(), &entry("a", "Payout stuck pending after KYC", "cleared the stale hold")).unwrap();
        let found = similar(home.path(), "My payout is stuck on pending since KYC passed", None, 3);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].resolution, "cleared the stale hold");
        assert!(render_history(&found).contains("Settled: cleared the stale hold"));
    }

    fn case(category: Category, confidence: u8, lookups: usize) -> Case {
        Case {
            ticket_id: "t1".into(),
            lookups: (0..lookups)
                .map(|_| Lookup { at: Utc::now(), source: "db".into(), request: "find accounts".into(), result: "1 document".into() })
                .collect(),
            investigation: Some(Investigation {
                at: Utc::now(),
                category,
                severity: Severity::Normal,
                confidence,
                findings: "The account's payout is held because KYC is pending.".into(),
                evidence: vec![],
                duplicate_of: None,
            }),
            ..Default::default()
        }
    }

    fn reply(kind: ReplyKind, content: &str) -> ReplyRequest {
        ReplyRequest { kind, content: content.into(), close: false, fix_pr: None, test: None }
    }

    fn no_pr(_: &str) -> Result<FixPr> {
        panic!("not looked up")
    }

    #[test]
    fn nothing_goes_out_before_an_investigation_with_evidence() {
        let empty = Case::default();
        assert!(check_reply(&empty, &reply(ReplyKind::Ack, "We're looking into it now."), &[], &no_pr).is_err());
        let unsupported = case(Category::Bug, 80, 0);
        let err = check_reply(&unsupported, &reply(ReplyKind::Ack, "We're looking into it now."), &[], &no_pr).unwrap_err();
        assert!(err.to_string().contains("no evidence"), "{err}");
        assert!(check_reply(&case(Category::Bug, 80, 1), &reply(ReplyKind::Ack, "We're looking into it now."), &[], &no_pr).is_ok());
    }

    #[test]
    fn answers_need_ninety_percent_and_requests_get_nothing() {
        let answer = reply(ReplyKind::Answer, "Payouts are processed within two working days.");
        assert!(check_reply(&case(Category::Question, 89, 1), &answer, &[], &no_pr).unwrap_err().to_string().contains("90%"));
        assert!(check_reply(&case(Category::Question, 90, 1), &answer, &[], &no_pr).is_ok());
        assert!(check_reply(&case(Category::Request, 100, 1), &answer, &[], &no_pr).unwrap_err().to_string().contains("no reply"));
    }

    #[test]
    fn resolved_needs_a_certain_micro_fix_that_is_merged_or_green() {
        let mut ask = reply(ReplyKind::Resolved, "This is fixed and goes out in the next update.");
        ask.fix_pr = Some("https://github.com/o/r/pull/9".into());
        ask.test = Some("payout_hold_clears_after_kyc reproduces it and passes".into());
        let sure = case(Category::Bug, 100, 2);
        let small = |_: &str| Ok(FixPr { state: "MERGED".into(), changed_lines: 12, checks_green: true });
        let big = |_: &str| Ok(FixPr { state: "MERGED".into(), changed_lines: 400, checks_green: true });
        let red = |_: &str| Ok(FixPr { state: "OPEN".into(), changed_lines: 12, checks_green: false });
        assert!(check_reply(&sure, &ask, &[], &small).is_ok());
        assert!(check_reply(&sure, &ask, &[], &big).unwrap_err().to_string().contains("micro"));
        assert!(check_reply(&sure, &ask, &[], &red).is_err());
        assert!(check_reply(&case(Category::Bug, 95, 2), &ask, &[], &small).unwrap_err().to_string().contains("certain"));
    }

    #[test]
    fn internal_details_are_kept_from_the_partner() {
        let ok = privacy_problems("Your payout went out this morning.", &[]);
        assert!(ok.is_empty(), "{ok:?}");
        assert!(!privacy_problems("Account 65f1c2a9b8e4d3f2a1b0c9d8 is held", &[]).is_empty());
        assert!(!privacy_problems("see src/payouts/hold.ts:42", &[]).is_empty());
        assert!(!privacy_problems("I checked bob@other.com's account", &["jo@acme.com".into()]).is_empty());
        assert!(privacy_problems("We emailed jo@acme.com", &["jo@acme.com".into()]).is_empty());
    }

    #[test]
    fn masked_secrets_survive_a_save() {
        let home = tempfile::tempdir().unwrap();
        let input = |url: &str, header: Option<&str>| SettingsInput {
            manager_project: Some("Propfirm".into()),
            auto_triage: true,
            reply_mode: ReplyMode::Approve,
            sources: vec![SourceInput {
                name: "db".into(),
                kind: SourceKind::Mongo,
                url: url.into(),
                header: header.map(str::to_string),
                notes: String::new(),
            }],
            kubernetes: false,
            kube_context: None,
        };
        let shown = save_settings(home.path(), input("mongodb://reader:hunter2@db:27017/prop", Some("X-Key: abc"))).unwrap();
        assert_eq!(shown.sources[0].url, "mongodb://reader:••••@db:27017/prop");
        save_settings(home.path(), input(&shown.sources[0].url, None)).unwrap();
        let saved = load_settings(home.path());
        assert_eq!(saved.sources[0].url, "mongodb://reader:hunter2@db:27017/prop");
        assert_eq!(saved.sources[0].header.as_deref(), Some("X-Key: abc"));
        assert!(saved.triage_since.is_some());
    }
}
