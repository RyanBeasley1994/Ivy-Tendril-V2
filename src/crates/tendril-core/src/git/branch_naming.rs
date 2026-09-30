//! How a plan's branch is named: the `git.branchTemplate` / `git.missionBranchTemplate` settings,
//! rendered once per plan and then recorded in its `plan.yaml`.
//!
//! **Recorded, not recomputed.** A branch name is read back by everything that touches a plan after
//! its worktree exists - `CreatePr` pushing it, `RetryPlan` resuming on it, the reaper deciding what
//! to delete, a mission fast-forwarding it. If each of those re-rendered the template, editing the
//! setting would silently point them all at a branch that does not exist. So the first worktree
//! creation stamps the rendered name under `branch:` in `plan.yaml`, and
//! [`crate::git::worktree::derive_branch_name`] prefers that stamp over everything else. A plan
//! created before this existed has no stamp and keeps the historical `tendril/<folder>`.

use crate::error::Result;
use crate::models::PlanYaml;
use crate::plans::reader::read_plan_yaml;
use crate::plans::writer::write_plan_yaml;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DEFAULT_BRANCH_PREFIX: &str = "tendril/";
pub const DEFAULT_BRANCH_TEMPLATE: &str = "{prefix}{folder}";
pub const DEFAULT_MISSION_BRANCH_TEMPLATE: &str = "{prefix}{folder}";

/// The `plan.yaml` key holding a plan's recorded branch name.
pub const PLAN_BRANCH_KEY: &str = "branch";

/// The tokens a template may use, for validation and for the Settings help text.
pub const BRANCH_TOKENS: [&str; 10] = [
    "prefix", "folder", "id", "title", "slug", "project", "level", "date", "mission", "milestone",
];

/// `git:` in `config.yaml`. Every field is optional; an absent block names branches exactly as
/// Tendril always has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSettings {
    /// Prepended wherever a template says `{prefix}`. Default `tendril/`.
    #[serde(rename = "branchPrefix", default, skip_serializing_if = "Option::is_none")]
    pub branch_prefix: Option<String>,
    /// The branch of an ordinary plan (and of a mission's milestone plans). Default
    /// `{prefix}{folder}`.
    #[serde(rename = "branchTemplate", default, skip_serializing_if = "Option::is_none")]
    pub branch_template: Option<String>,
    /// The branch of a mission's integration plan: the mission branch every milestone lands on and
    /// the one its pull request is opened from. Default `{prefix}{folder}`.
    #[serde(rename = "missionBranchTemplate", default, skip_serializing_if = "Option::is_none")]
    pub mission_branch_template: Option<String>,
    /// `false` turns commit signing off for the commits agents make (`commit.gpgsign=false` in their
    /// environment only). For a signing setup that needs a person present - a 1Password SSH key, a
    /// GPG pinentry - which cannot sign in a headless run. Absent or `true`: your git config applies.
    #[serde(rename = "signCommits", default, skip_serializing_if = "Option::is_none")]
    pub sign_commits: Option<bool>,
    /// `false` sends every Create PR through the `CreatePr` agent, as before. Absent or `true`: Tendril
    /// pushes and opens the PR itself and only calls the agent for merge conflicts and odd cases.
    #[serde(rename = "nativePullRequests", default, skip_serializing_if = "Option::is_none")]
    pub native_pull_requests: Option<bool>,
}

impl GitSettings {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Whether any naming field is set. Signing does not name branches, so it must not cause a
    /// branch to be recorded.
    pub fn names_branches(&self) -> bool {
        self.branch_prefix.is_some()
            || self.branch_template.is_some()
            || self.mission_branch_template.is_some()
    }

    fn prefix(&self) -> &str {
        self.branch_prefix.as_deref().unwrap_or(DEFAULT_BRANCH_PREFIX)
    }
}

/// What a template is rendered from.
#[derive(Debug, Clone, Default)]
pub struct BranchContext {
    /// `00042-AddLogin`.
    pub folder: String,
    pub project: String,
    pub level: String,
    /// The mission id (`00003`), for a mission's plans.
    pub mission: Option<String>,
    /// The milestone id (`M2`), for a milestone plan.
    pub milestone: Option<String>,
}

impl BranchContext {
    pub fn from_plan(folder: &Path, plan: &PlanYaml) -> Self {
        let link = crate::missions::model::mission_link(plan);
        Self {
            folder: folder
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("plan")
                .to_string(),
            project: plan.project.clone(),
            level: plan.level.clone(),
            mission: link.as_ref().and_then(|l| {
                Path::new(&l.folder)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.chars().take(5).collect())
            }),
            milestone: link.and_then(|l| l.milestone),
        }
    }
}

/// `AddLogin` → `add-login`; anything that is not alphanumeric becomes a single dash.
fn kebab(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && prev_lower {
                out.push('-');
            }
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
            out.push(ch.to_ascii_lowercase());
        } else {
            if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('-').to_string()
}

/// Makes `name` a valid git branch name (`git check-ref-format --branch` rules): no spaces or control
/// characters, none of `~^:?*[\`, no `..`, `@{`, `//`, no leading or trailing `/` or `.`, no
/// `.lock` component ending, and a sane length. Falls back to `fallback` if nothing is left.
pub fn sanitize_branch_name(name: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let bad = ch.is_control() || ch.is_whitespace() || "~^:?*[\\".contains(ch);
        out.push(if bad { '-' } else { ch });
    }
    while out.contains("..") {
        out = out.replace("..", ".");
    }
    out = out.replace("@{", "-");
    while out.contains("//") {
        out = out.replace("//", "/");
    }
    let parts: Vec<String> = out
        .split('/')
        .map(|p| {
            let p = p.trim_matches('.').trim_matches('-');
            p.strip_suffix(".lock").unwrap_or(p).to_string()
        })
        .filter(|p| !p.is_empty())
        .collect();
    let mut joined = parts.join("/");
    if joined.len() > 200 {
        joined.truncate(200);
        joined = joined.trim_end_matches(['/', '.', '-']).to_string();
    }
    if joined.is_empty() || joined == "@" {
        fallback.to_string()
    } else {
        joined
    }
}

/// The tokens a template names that are not known, so Settings can refuse a typo before it names a
/// branch `feature/{tittle}`.
pub fn unknown_tokens(template: &str) -> Vec<String> {
    let mut unknown = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let token = &after[..end];
        if !BRANCH_TOKENS.contains(&token) && !unknown.iter().any(|u| u == token) {
            unknown.push(token.to_string());
        }
        rest = &after[end + 1..];
    }
    unknown
}

/// Renders a branch name. `mission` selects the mission template (integration plans).
pub fn render_branch_name(settings: &GitSettings, ctx: &BranchContext, mission: bool) -> String {
    let template = if mission {
        settings
            .mission_branch_template
            .as_deref()
            .unwrap_or(DEFAULT_MISSION_BRANCH_TEMPLATE)
    } else {
        settings.branch_template.as_deref().unwrap_or(DEFAULT_BRANCH_TEMPLATE)
    };
    let id: String = ctx.folder.chars().take(5).collect();
    let title = ctx.folder.get(6..).unwrap_or("").to_string();
    let rendered = template
        .replace("{prefix}", settings.prefix())
        .replace("{folder}", &ctx.folder)
        .replace("{id}", &id)
        .replace("{title}", &title)
        .replace("{slug}", &kebab(&title))
        .replace("{project}", &kebab(&ctx.project))
        .replace("{level}", &kebab(&ctx.level))
        .replace("{date}", &chrono::Local::now().format("%Y%m%d").to_string())
        .replace("{mission}", ctx.mission.as_deref().unwrap_or(""))
        .replace("{milestone}", &ctx.milestone.as_deref().unwrap_or("").to_ascii_lowercase());
    sanitize_branch_name(&rendered, &format!("{}{}", DEFAULT_BRANCH_PREFIX, ctx.folder))
}

/// The branch recorded for a plan, if one has been.
pub fn recorded_branch(plan: &PlanYaml) -> Option<String> {
    plan.extra
        .get(PLAN_BRANCH_KEY)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_string)
}

/// The plan's branch name: the recorded one if there is one; otherwise rendered from `settings`
/// and recorded now, so it never changes again. With default settings nothing is recorded - the
/// historical `tendril/<folder>` is what [`crate::git::worktree::derive_branch_name`] answers anyway,
/// and leaving `plan.yaml` untouched keeps every existing install's files as they were.
pub fn assign_branch_name(plan_folder: &Path, settings: &GitSettings) -> Result<String> {
    let (mut plan, _) = read_plan_yaml(plan_folder)?;
    if let Some(existing) = recorded_branch(&plan) {
        return Ok(existing);
    }
    let is_integration = crate::missions::model::mission_link(&plan)
        .is_some_and(|l| l.role == crate::missions::model::MissionRole::Integration);
    let ctx = BranchContext::from_plan(plan_folder, &plan);
    let name = render_branch_name(settings, &ctx, is_integration);
    if !settings.names_branches() {
        return Ok(name);
    }
    plan.extra.insert(
        PLAN_BRANCH_KEY.to_string(),
        serde_yaml::Value::String(name.clone()),
    );
    write_plan_yaml(plan_folder, &plan)?;
    Ok(name)
}

/// What each kind of branch would be called under `settings`, for the Settings preview. Rendered here
/// rather than in the app so the preview can never disagree with what a plan actually gets.
#[derive(Debug, Clone, Serialize)]
pub struct BranchPreview {
    pub plan: String,
    pub milestone: String,
    pub mission: String,
    #[serde(rename = "unknownTokens")]
    pub unknown_tokens: Vec<String>,
}

pub fn preview_branch_names(settings: &GitSettings, project: &str) -> BranchPreview {
    let plan = BranchContext {
        folder: "00042-AddSsoLogin".into(),
        project: project.into(),
        level: "Feature".into(),
        mission: None,
        milestone: None,
    };
    let milestone = BranchContext {
        folder: "00043-M2SessionStore".into(),
        mission: Some("00003".into()),
        milestone: Some("M2".into()),
        ..plan.clone()
    };
    let mission = BranchContext {
        folder: "00041-AddSso".into(),
        level: "Epic".into(),
        mission: Some("00003".into()),
        ..plan.clone()
    };
    let mut unknown = Vec::new();
    for template in [&settings.branch_template, &settings.mission_branch_template]
        .into_iter()
        .flatten()
    {
        for token in unknown_tokens(template) {
            if !unknown.contains(&token) {
                unknown.push(token);
            }
        }
    }
    BranchPreview {
        plan: render_branch_name(settings, &plan, false),
        milestone: render_branch_name(settings, &milestone, false),
        mission: render_branch_name(settings, &mission, true),
        unknown_tokens: unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> BranchContext {
        BranchContext {
            folder: "00042-AddSsoLogin".into(),
            project: "My App".into(),
            level: "Feature".into(),
            mission: None,
            milestone: None,
        }
    }

    #[test]
    fn default_settings_keep_the_historical_name() {
        assert_eq!(render_branch_name(&GitSettings::default(), &ctx(), false), "tendril/00042-AddSsoLogin");
        assert_eq!(render_branch_name(&GitSettings::default(), &ctx(), true), "tendril/00042-AddSsoLogin");
    }

    #[test]
    fn tokens_render() {
        let s = GitSettings {
            branch_prefix: Some("rb/".into()),
            branch_template: Some("{prefix}{level}/{id}-{slug}".into()),
            mission_branch_template: Some("{prefix}mission-{mission}-{slug}".into()),
            sign_commits: None,
            native_pull_requests: None,
        };
        assert_eq!(render_branch_name(&s, &ctx(), false), "rb/feature/00042-add-sso-login");
        let mut c = ctx();
        c.mission = Some("00003".into());
        c.milestone = Some("M2".into());
        assert_eq!(render_branch_name(&s, &c, true), "rb/mission-00003-add-sso-login");
        let s = GitSettings { branch_template: Some("{project}/{milestone}-{title}".into()), ..Default::default() };
        assert_eq!(render_branch_name(&s, &c, false), "my-app/m2-AddSsoLogin");
    }

    #[test]
    fn names_are_made_valid_refs() {
        assert_eq!(sanitize_branch_name("feat//a b..c~d.lock/", "x"), "feat/a-b.c-d");
        assert_eq!(sanitize_branch_name("/.hidden/", "x"), "hidden");
        assert_eq!(sanitize_branch_name("", "tendril/x"), "tendril/x");
        // An empty token must not leave a dangling separator.
        let s = GitSettings { branch_template: Some("{prefix}{mission}/{id}".into()), ..Default::default() };
        assert_eq!(render_branch_name(&s, &ctx(), false), "tendril/00042");
    }

    #[test]
    fn unknown_tokens_are_reported() {
        assert_eq!(unknown_tokens("{prefix}{tittle}-{id}{x}{x}"), vec!["tittle", "x"]);
        assert!(unknown_tokens("{prefix}{folder}").is_empty());
    }

    #[test]
    fn a_branch_is_recorded_once_and_survives_a_template_change() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("00042-AddSsoLogin");
        std::fs::create_dir_all(&folder).unwrap();
        write_plan_yaml(&folder, &PlanYaml::default()).unwrap();

        // Default settings record nothing, and neither does a signing-only setting.
        let signing_only = GitSettings { sign_commits: Some(false), ..Default::default() };
        assert_eq!(assign_branch_name(&folder, &signing_only).unwrap(), "tendril/00042-AddSsoLogin");
        assert!(recorded_branch(&read_plan_yaml(&folder).unwrap().0).is_none());
        assert_eq!(assign_branch_name(&folder, &GitSettings::default()).unwrap(), "tendril/00042-AddSsoLogin");
        assert!(recorded_branch(&read_plan_yaml(&folder).unwrap().0).is_none());

        let first = GitSettings { branch_template: Some("feat/{slug}".into()), ..Default::default() };
        assert_eq!(assign_branch_name(&folder, &first).unwrap(), "feat/add-sso-login");
        let changed = GitSettings { branch_template: Some("other/{id}".into()), ..Default::default() };
        assert_eq!(assign_branch_name(&folder, &changed).unwrap(), "feat/add-sso-login");
        assert_eq!(crate::git::worktree::derive_branch_name(&folder), "feat/add-sso-login");
    }
}
