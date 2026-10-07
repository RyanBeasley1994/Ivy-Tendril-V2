//! The commit graph: `git log` in topological order, and the lanes a drawing needs.
//!
//! The layout is computed here rather than in the browser because it is the one part of the page where
//! being subtly wrong is easy and invisible, and a Rust test against real merges is cheap. A row says
//! where its dot sits and which line segments cross it; the page draws them and does no geometry of its
//! own beyond mapping lane numbers to x positions.

use super::{git_read_ok, validate_ref_name};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphRefKind {
    Head,
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRef {
    pub name: String,
    pub kind: GraphRefKind,
    /// The checked-out branch (or HEAD itself, when detached).
    pub is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphCommit {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Unix seconds.
    pub at: i64,
    pub subject: String,
    pub refs: Vec<GraphRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinePart {
    /// Passes straight through the row, top to bottom.
    Full,
    /// From the top edge of the row into the dot.
    Top,
    /// From the dot to the bottom edge of the row.
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphLine {
    pub from: u32,
    pub to: u32,
    pub color: u32,
    pub part: LinePart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    #[serde(flatten)]
    pub commit: GraphCommit,
    pub lane: u32,
    pub color: u32,
    pub lines: Vec<GraphLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub rows: Vec<GraphRow>,
    /// The most lanes any row uses: how wide the drawing has to be.
    pub width: u32,
    /// There are older commits than the ones returned.
    pub more: bool,
}

const PALETTE: u32 = 8;

/// FNV-1a, for colours that do not change from one look at the repository to the next.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// A colour for a new lane: the one its seed hashes to, moved on past any colour a lane that is still open
/// is already using, so neighbours are told apart. The same branch gets the same colour every time the
/// history grows, which is what makes the graph readable from one visit to the next.
fn pick_color(seed: &str, open: &[Option<Lane>]) -> u32 {
    let start = (fnv(seed) % PALETTE as u64) as u32;
    (0..PALETTE)
        .map(|i| (start + i) % PALETTE)
        .find(|c| !open.iter().flatten().any(|l| l.color == *c))
        .unwrap_or(start)
}

/// What names the lane a tip starts: its branch (a remote branch counts as the branch it tracks), else a
/// tag, else the commit itself.
fn tip_seed(commit: &GraphCommit) -> String {
    commit
        .refs
        .iter()
        .find(|r| r.kind == GraphRefKind::Local)
        .map(|r| r.name.clone())
        .or_else(|| {
            commit
                .refs
                .iter()
                .find(|r| r.kind == GraphRefKind::Remote)
                .map(|r| r.name.split_once('/').map(|(_, b)| b).unwrap_or(&r.name).to_string())
        })
        .or_else(|| commit.refs.iter().find(|r| r.kind == GraphRefKind::Tag).map(|r| r.name.clone()))
        .unwrap_or_else(|| commit.hash.clone())
}

#[derive(Clone)]
struct Lane {
    expects: String,
    color: u32,
}

/// Lays commits (children before parents, as `--topo-order` gives them) out in lanes.
pub fn assign_lanes(commits: Vec<GraphCommit>) -> Graph {
    let mut active: Vec<Option<Lane>> = Vec::new();
    let mut width = 0u32;
    let mut rows = Vec::with_capacity(commits.len());

    let free_slot = |active: &mut Vec<Option<Lane>>| -> usize {
        if let Some(i) = active.iter().position(Option::is_none) {
            i
        } else {
            active.push(None);
            active.len() - 1
        }
    };

    for commit in commits {
        let expecting: Vec<usize> = active
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_ref().is_some_and(|l| l.expects == commit.hash))
            .map(|(i, _)| i)
            .collect();

        let (lane, color) = match expecting.first() {
            Some(&i) => (i, active[i].as_ref().map(|l| l.color).unwrap_or(0)),
            None => {
                // A tip nothing was waiting for: a branch head, or a commit only reachable from HEAD.
                let i = free_slot(&mut active);
                (i, pick_color(&tip_seed(&commit), &active))
            }
        };

        let mut lines = Vec::new();
        for (i, l) in active.iter().enumerate() {
            if let Some(l) = l {
                if expecting.contains(&i) {
                    lines.push(GraphLine { from: i as u32, to: lane as u32, color: l.color, part: LinePart::Top });
                } else {
                    lines.push(GraphLine { from: i as u32, to: i as u32, color: l.color, part: LinePart::Full });
                }
            }
        }
        for &i in &expecting {
            active[i] = None;
        }
        let mut row_width = active.len().max(lane + 1);

        for (k, parent) in commit.parents.iter().enumerate() {
            let waiting = active.iter().position(|l| l.as_ref().is_some_and(|l| &l.expects == parent));
            match waiting {
                // Someone else already reaches this parent: join their lane instead of starting another.
                Some(j) => {
                    let c = active[j].as_ref().map(|l| l.color).unwrap_or(color);
                    lines.push(GraphLine { from: lane as u32, to: j as u32, color: c, part: LinePart::Bottom });
                }
                None if k == 0 => {
                    if lane >= active.len() {
                        active.resize(lane + 1, None);
                    }
                    active[lane] = Some(Lane { expects: parent.clone(), color });
                    lines.push(GraphLine { from: lane as u32, to: lane as u32, color, part: LinePart::Bottom });
                }
                None => {
                    let s = free_slot(&mut active);
                    let c = pick_color(parent, &active);
                    active[s] = Some(Lane { expects: parent.clone(), color: c });
                    lines.push(GraphLine { from: lane as u32, to: s as u32, color: c, part: LinePart::Bottom });
                    row_width = row_width.max(s + 1);
                }
            }
        }
        while active.last().is_some_and(Option::is_none) {
            active.pop();
        }
        width = width.max(row_width as u32).max(active.len() as u32);
        rows.push(GraphRow { commit, lane: lane as u32, color, lines });
    }

    Graph { rows, width, more: false }
}

fn parse_refs(decorations: &str) -> Vec<GraphRef> {
    let mut refs = Vec::new();
    for item in decorations.split(", ").map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(branch) = item.strip_prefix("HEAD -> ") {
            refs.push(GraphRef { name: "HEAD".into(), kind: GraphRefKind::Head, is_head: true });
            refs.push(GraphRef { name: branch.to_string(), kind: GraphRefKind::Local, is_head: true });
        } else if item == "HEAD" {
            refs.push(GraphRef { name: "HEAD".into(), kind: GraphRefKind::Head, is_head: true });
        } else if let Some(tag) = item.strip_prefix("tag: ") {
            refs.push(GraphRef { name: tag.to_string(), kind: GraphRefKind::Tag, is_head: false });
        } else if item.contains('/') && !item.ends_with("/HEAD") {
            // `origin/main` is a remote branch; a local branch with a slash in its name (`feature/x`) is
            // indistinguishable here, so the page treats anything it also lists as local as local.
            refs.push(GraphRef { name: item.to_string(), kind: GraphRefKind::Remote, is_head: false });
        } else if !item.ends_with("/HEAD") {
            refs.push(GraphRef { name: item.to_string(), kind: GraphRefKind::Local, is_head: false });
        }
    }
    refs
}

pub fn parse_log(output: &str) -> Vec<GraphCommit> {
    output
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\u{1f}').collect();
            if f.len() < 6 {
                return None;
            }
            Some(GraphCommit {
                hash: f[0].to_string(),
                short: f[0].chars().take(7).collect(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                author: f[2].to_string(),
                at: f[3].parse().unwrap_or(0),
                subject: f[5].to_string(),
                refs: parse_refs(f[4]),
            })
        })
        .collect()
}

/// The newest `limit` commits across every branch, remote branch, tag and HEAD, laid out in lanes.
/// `only_branch` narrows it to one branch's history.
pub fn read_graph(repo: &Path, limit: usize, only_branch: Option<&str>) -> Result<Graph> {
    let limit = limit.clamp(1, 5000);
    let count = format!("-n{}", limit + 1);
    let mut args: Vec<String> = vec![
        "log".into(),
        "--topo-order".into(),
        "--decorate=short".into(),
        count,
        "--format=%H%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s".into(),
    ];
    match only_branch {
        Some(branch) => {
            validate_ref_name(branch)?;
            args.push(branch.to_string());
            args.push("--".into());
        }
        None => {
            // Not `--all`: that would draw the stash's internal commits as if they were history.
            args.extend(["--branches", "--remotes", "--tags", "HEAD"].map(String::from));
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // An empty repository has no HEAD to log; that is an empty graph, not an error.
    let output = match git_read_ok(repo, &arg_refs) {
        Ok(o) => o,
        Err(_) => String::new(),
    };
    let mut commits = parse_log(&output);
    let more = commits.len() > limit;
    commits.truncate(limit);
    let mut graph = assign_lanes(commits);
    graph.more = more;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::super::testkit::Scratch;
    use super::*;

    fn c(hash: &str, parents: &[&str]) -> GraphCommit {
        GraphCommit {
            hash: hash.into(),
            short: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: "t".into(),
            at: 0,
            subject: hash.into(),
            refs: vec![],
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let g = assign_lanes(vec![c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert!(g.rows.iter().all(|r| r.lane == 0));
        assert_eq!(g.width, 1);
        // Each interior row has a line coming in and a line going out in lane 0, and nothing else.
        assert_eq!(g.rows[1].lines.len(), 2);
        assert_eq!(g.rows[0].lines.len(), 1, "the newest commit has nothing above it");
    }

    #[test]
    fn a_branch_and_its_merge_use_two_lanes_and_rejoin() {
        //   m  (merge of f and b)
        //   |\
        //   f |   feature commit
        //   | b   main commit
        //   |/
        //   a
        let g = assign_lanes(vec![c("m", &["b", "f"]), c("f", &["a"]), c("b", &["a"]), c("a", &[])]);
        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 1, "the feature commit sits in the second lane");
        assert_eq!(g.rows[2].lane, 0);
        assert_eq!(g.width, 2);
        // The merge sends one line down its own lane and one across to the feature lane.
        let m = &g.rows[0];
        assert!(m.lines.iter().any(|l| l.part == LinePart::Bottom && l.from == 0 && l.to == 0));
        assert!(m.lines.iter().any(|l| l.part == LinePart::Bottom && l.from == 0 && l.to == 1));
        // `b` sits on main's lane, but `a` is already awaited on the feature lane, so `b` curves across into
        // it in its own row, and `a` is then reached by that single lane.
        let b = &g.rows[2];
        assert!(b.lines.iter().any(|l| l.part == LinePart::Bottom && l.from == 0 && l.to == 1));
        let a = &g.rows[3];
        assert_eq!(a.lane, 1);
        assert_eq!(a.lines.iter().filter(|l| l.part == LinePart::Top).count(), 1);
    }

    #[test]
    fn two_branch_tips_get_different_lanes_and_colours_and_an_octopus_merge_spreads_out() {
        let g = assign_lanes(vec![c("x", &["a"]), c("y", &["a"]), c("a", &[])]);
        assert_ne!(g.rows[0].lane, g.rows[1].lane);
        assert_ne!(g.rows[0].color, g.rows[1].color);

        let octopus = assign_lanes(vec![c("m", &["p1", "p2", "p3"]), c("p1", &[]), c("p2", &[]), c("p3", &[])]);
        assert_eq!(octopus.rows[0].lines.iter().filter(|l| l.part == LinePart::Bottom).count(), 3);
        assert_eq!(octopus.width, 3);
    }

    #[test]
    fn a_branch_keeps_its_colour_when_new_commits_land_on_top() {
        let on = |hash: &str, parents: &[&str], branch: Option<&str>| {
            let mut commit = c(hash, parents);
            if let Some(b) = branch {
                commit.refs.push(GraphRef { name: b.into(), kind: GraphRefKind::Local, is_head: false });
            }
            commit
        };
        let before = assign_lanes(vec![on("b", &["a"], Some("main")), on("a", &[], None)]);
        let after = assign_lanes(vec![on("c", &["b"], Some("main")), on("b", &["a"], None), on("a", &[], None)]);
        assert_eq!(before.rows[0].color, after.rows[0].color, "main is the same colour with one more commit");
        assert_eq!(after.rows[0].color, after.rows[2].color, "and it is one colour all the way down");
    }

    #[test]
    fn decorations_become_typed_refs() {
        let refs = parse_refs("HEAD -> main, tag: v1.0, origin/main, origin/HEAD, feature");
        let names: Vec<(&str, GraphRefKind)> = refs.iter().map(|r| (r.name.as_str(), r.kind)).collect();
        assert_eq!(
            names,
            vec![
                ("HEAD", GraphRefKind::Head),
                ("main", GraphRefKind::Local),
                ("v1.0", GraphRefKind::Tag),
                ("origin/main", GraphRefKind::Remote),
                ("feature", GraphRefKind::Local),
            ]
        );
        assert!(refs[1].is_head);
    }

    #[test]
    fn a_real_branch_and_merge_come_back_laid_out_with_labels_and_paging() {
        let repo = Scratch::new();
        repo.commit("a.txt", "1\n", "base");
        repo.git(&["checkout", "-q", "-b", "feature"]);
        repo.commit("f.txt", "f\n", "feature work");
        repo.git(&["checkout", "-q", "main"]);
        repo.commit("m.txt", "m\n", "main work");
        repo.git(&["merge", "-q", "--no-ff", "-m", "merge feature", "feature"]);
        repo.git(&["tag", "v1"]);

        let g = read_graph(&repo.dir, 50, None).unwrap();
        assert_eq!(g.rows.len(), 4);
        assert!(!g.more);
        assert_eq!(g.rows[0].commit.subject, "merge feature");
        assert_eq!(g.rows[0].commit.parents.len(), 2);
        assert!(g.rows[0].commit.refs.iter().any(|r| r.name == "v1" && r.kind == GraphRefKind::Tag));
        assert!(g.rows[0].commit.refs.iter().any(|r| r.name == "main" && r.is_head));
        assert!(g.width >= 2, "a real merge needs two lanes");

        let page = read_graph(&repo.dir, 2, None).unwrap();
        assert_eq!(page.rows.len(), 2);
        assert!(page.more, "older commits exist beyond the page");

        let only = read_graph(&repo.dir, 50, Some("feature")).unwrap();
        assert_eq!(only.rows.len(), 2, "feature's own history: its commit and the base");
        assert!(read_graph(&repo.dir, 10, Some("--all")).is_err(), "an option is not a branch name");
    }

    #[test]
    fn an_empty_repository_has_an_empty_graph() {
        let repo = Scratch::new();
        let g = read_graph(&repo.dir, 10, None).unwrap();
        assert!(g.rows.is_empty() && !g.more);
    }
}
