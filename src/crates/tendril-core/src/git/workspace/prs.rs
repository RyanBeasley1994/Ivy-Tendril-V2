//! Pull requests for a repository, through `gh`: which branches have one and how its checks stand, plus
//! the title and body a new one starts from.

use super::{gh, git_read_ok, resolve_commit, validate_ref_name, READ_TIMEOUT};
use crate::error::{Result, TendrilError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChecksState {
    /// Every check finished and none failed.
    Passing,
    Failing,
    Pending,
    /// No checks at all.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrBrief {
    pub number: u64,
    pub title: String,
    /// The branch the PR is made from.
    pub branch: String,
    pub base: String,
    /// `OPEN`, `MERGED` or `CLOSED`.
    pub state: String,
    pub url: String,
    pub draft: bool,
    pub checks: ChecksState,
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED` or empty.
    pub review: String,
}

/// Collapses `statusCheckRollup` to one word. A running or queued check outweighs a finished failure,
/// because the answer is not known until it ends.
pub fn checks_state(rollup: &Value) -> ChecksState {
    let Some(items) = rollup.as_array().filter(|a| !a.is_empty()) else {
        return ChecksState::None;
    };
    let (mut pending, mut failing) = (false, false);
    for c in items {
        if let Some(status) = c["status"].as_str() {
            if !status.eq_ignore_ascii_case("COMPLETED") {
                pending = true;
            } else if matches!(
                c["conclusion"].as_str().unwrap_or("").to_ascii_uppercase().as_str(),
                "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
            ) {
                failing = true;
            }
        } else {
            match c["state"].as_str().unwrap_or("").to_ascii_uppercase().as_str() {
                "PENDING" | "EXPECTED" => pending = true,
                "FAILURE" | "ERROR" => failing = true,
                _ => {}
            }
        }
    }
    if pending {
        ChecksState::Pending
    } else if failing {
        ChecksState::Failing
    } else {
        ChecksState::Passing
    }
}

pub fn parse_prs(json: &str) -> Vec<PrBrief> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|p| {
            Some(PrBrief {
                number: p["number"].as_u64()?,
                title: p["title"].as_str().unwrap_or("").to_string(),
                branch: p["headRefName"].as_str()?.to_string(),
                base: p["baseRefName"].as_str().unwrap_or("").to_string(),
                state: p["state"].as_str().unwrap_or("OPEN").to_string(),
                url: p["url"].as_str().unwrap_or("").to_string(),
                draft: p["isDraft"].as_bool().unwrap_or(false),
                checks: checks_state(&p["statusCheckRollup"]),
                author: p["author"]["login"].as_str().unwrap_or("").to_string(),
                review: p["reviewDecision"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Open pull requests, or `Err` with a reason a person can act on (gh missing, not signed in, no GitHub
/// remote). The page shows that as a quiet note, not as a failure.
pub fn list_open_prs(repo: &Path) -> Result<Vec<PrBrief>> {
    let out = gh(
        repo,
        &[
            "pr", "list", "--state", "open", "--limit", "100", "--json",
            "number,title,headRefName,baseRefName,state,url,isDraft,statusCheckRollup,reviewDecision,author",
        ],
        READ_TIMEOUT,
    )?;
    if !out.ok() {
        return Err(TendrilError::Git(out.message()));
    }
    Ok(parse_prs(&out.stdout))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrPrefill {
    pub title: String,
    pub body: String,
    /// Subjects of the commits the PR would contain, newest first.
    pub commits: Vec<String>,
}

/// A starting title and body for a PR of `head` into `base`: the lone commit's subject (or the branch name
/// made readable) as the title, and the commit subjects as a list.
pub fn prefill_pr(repo: &Path, head: &str, base: &str) -> Result<PrPrefill> {
    validate_ref_name(head)?;
    validate_ref_name(base)?;
    let head_hash = resolve_commit(repo, head)?;
    let base_hash = resolve_commit(repo, base)?;
    let range = format!("{base_hash}..{head_hash}");
    let log = git_read_ok(repo, &["log", "--format=%s%x1f%b%x1e", &range])?;
    let mut commits = Vec::new();
    let mut bodies = Vec::new();
    for record in log.split('\u{1e}').map(str::trim).filter(|r| !r.is_empty()) {
        let mut parts = record.splitn(2, '\u{1f}');
        let subject = parts.next().unwrap_or("").trim().to_string();
        let body = parts.next().unwrap_or("").trim().to_string();
        if !subject.is_empty() {
            commits.push(subject);
        }
        if !body.is_empty() {
            bodies.push(body);
        }
    }
    let title = if commits.len() == 1 {
        commits[0].clone()
    } else {
        // `tendril/00042-FixLogin` or `feature/add-search` becomes "Fix login" / "Add search".
        let last = head.rsplit('/').next().unwrap_or(head);
        let last = last.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
        let spaced = last.replace(['-', '_'], " ");
        let mut chars = spaced.chars();
        chars.next().map(|f| f.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_else(|| head.to_string())
    };
    let mut body = String::new();
    if commits.len() > 1 {
        body.push_str("## Changes\n\n");
        for c in &commits {
            body.push_str(&format!("- {c}\n"));
        }
    } else if let Some(b) = bodies.first() {
        body.push_str(b);
        body.push('\n');
    }
    Ok(PrPrefill { title, body, commits })
}

/// Creates the PR with `gh`, from the repository so it picks up the right GitHub remote. Returns its URL.
pub fn create_pr_for(repo: &Path, head: &str, base: &str, title: &str, body: &str, draft: bool) -> Result<String> {
    validate_ref_name(head)?;
    validate_ref_name(base)?;
    if title.trim().is_empty() {
        return Err(TendrilError::Validation("A pull request needs a title.".into()));
    }
    let mut args = vec!["pr", "create", "--head", head, "--base", base, "--title", title.trim(), "--body", body];
    if draft {
        args.push("--draft");
    }
    let out = gh(repo, &args, super::NETWORK_TIMEOUT)?;
    if !out.ok() {
        return Err(TendrilError::Git(out.message()));
    }
    Ok(out.stdout.lines().rev().find(|l| l.starts_with("http")).unwrap_or(out.stdout.trim()).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::super::testkit::Scratch;
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_collapse_to_one_state_with_running_outweighing_failed() {
        assert_eq!(checks_state(&json!([])), ChecksState::None);
        assert_eq!(checks_state(&Value::Null), ChecksState::None);
        let ok = json!([{"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"COMPLETED","conclusion":"SKIPPED"}]);
        assert_eq!(checks_state(&ok), ChecksState::Passing);
        let failing = json!([{"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"COMPLETED","conclusion":"FAILURE"}]);
        assert_eq!(checks_state(&failing), ChecksState::Failing);
        let running = json!([{"status":"COMPLETED","conclusion":"FAILURE"},{"status":"IN_PROGRESS"}]);
        assert_eq!(checks_state(&running), ChecksState::Pending);
        assert_eq!(checks_state(&json!([{"state":"ERROR"}])), ChecksState::Failing);
    }

    #[test]
    fn gh_json_becomes_pr_briefs_and_junk_is_ignored() {
        let prs = parse_prs(
            r#"[{"number":46,"title":"Module abstraction","headRefName":"feature/modules","baseRefName":"main",
                 "state":"OPEN","url":"https://github.com/o/r/pull/46","isDraft":true,
                 "statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}],
                 "reviewDecision":"APPROVED","author":{"login":"ryan"}},{"title":"no number"}]"#,
        );
        assert_eq!(prs.len(), 1);
        let p = &prs[0];
        assert_eq!((p.number, p.branch.as_str(), p.draft, p.checks), (46, "feature/modules", true, ChecksState::Passing));
        assert_eq!((p.author.as_str(), p.review.as_str()), ("ryan", "APPROVED"));
        assert!(parse_prs("not json").is_empty());
    }

    #[test]
    fn a_pr_starts_from_the_commits_it_will_contain() {
        let repo = Scratch::new();
        repo.commit("a.txt", "1\n", "base");
        repo.git(&["checkout", "-q", "-b", "tendril/00042-fix-login-form"]);
        let one = repo.commit("a.txt", "2\n", "Fix the login form");
        let single = prefill_pr(&repo.dir, "tendril/00042-fix-login-form", "main").unwrap();
        assert_eq!(single.title, "Fix the login form");
        assert_eq!(single.commits, vec!["Fix the login form".to_string()]);
        let _ = one;

        repo.commit("b.txt", "x\n", "Add the validation");
        let many = prefill_pr(&repo.dir, "tendril/00042-fix-login-form", "main").unwrap();
        assert_eq!(many.title, "Fix login form", "the branch name, made readable, when there are several commits");
        assert!(many.body.contains("- Add the validation") && many.body.contains("- Fix the login form"));
        assert!(prefill_pr(&repo.dir, "--all", "main").is_err());
        assert!(create_pr_for(&repo.dir, "main", "main", "  ", "", false).is_err(), "an empty title is refused before gh runs");
    }
}
