//! The GitHub repositories the signed-in `gh` user can clone, for the Add Project picker.
//!
//! One `gh api user/repos` call covers every affiliation — the user's own repositories, ones they
//! collaborate on, and their organizations' — already sorted most recently updated first, which is
//! the order someone picking "the repo I was just working on" wants. `--paginate` follows every page,
//! so an account with hundreds of repositories is listed whole and the picker's search runs over all
//! of them rather than the first page.

use crate::error::{Result, TendrilError};
use crate::git::issues::run_gh_command_raw;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRepo {
    /// `owner/name`.
    pub full_name: String,
    pub owner: String,
    pub name: String,
    /// What a clone is given: the HTTPS URL, which `gh auth setup-git` makes work for private repos.
    pub clone_url: String,
    #[serde(default)]
    pub description: Option<String>,
    pub is_private: bool,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// One JSON object per line, as `--jq` prints it.
const REPO_JQ: &str = ".[] | {fullName: .full_name, owner: .owner.login, name: .name, \
     cloneUrl: .clone_url, description: .description, isPrivate: .private, updatedAt: .updated_at}";

pub async fn list_github_repos() -> Result<Vec<GitHubRepo>> {
    let args: Vec<String> = [
        "api",
        "--paginate",
        "user/repos?per_page=100&sort=updated&affiliation=owner,collaborator,organization_member",
        "--jq",
        REPO_JQ,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let (code, stdout, stderr) = run_gh_command_raw(&args, None).await.map_err(|e| {
        TendrilError::Other(format!(
            "Could not run the GitHub CLI (gh). Install it and run `gh auth login`: {e}"
        ))
    })?;
    if code != 0 {
        let detail = stderr.trim();
        return Err(TendrilError::Other(
            if detail.contains("auth login") || detail.contains("not logged") {
                "GitHub CLI is not signed in. Run `gh auth login` on the machine Tendril runs on."
                    .to_string()
            } else {
                format!("GitHub CLI failed listing repositories: {detail}")
            },
        ));
    }
    Ok(parse_repo_lines(&stdout))
}

/// Lines that do not parse are skipped: one odd row is not worth losing the list over.
fn parse_repo_lines(stdout: &str) -> Vec<GitHubRepo> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<GitHubRepo>(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jq_lines_and_skips_junk() {
        let out = r#"{"fullName":"me/app","owner":"me","name":"app","cloneUrl":"https://github.com/me/app.git","description":null,"isPrivate":true,"updatedAt":"2026-09-30T10:00:00Z"}
not json
{"fullName":"org/lib","owner":"org","name":"lib","cloneUrl":"https://github.com/org/lib.git","description":"A lib","isPrivate":false,"updatedAt":null}
"#;
        let repos = parse_repo_lines(out);
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].full_name, "me/app");
        assert!(repos[0].is_private);
        assert_eq!(repos[1].description.as_deref(), Some("A lib"));
    }
}
