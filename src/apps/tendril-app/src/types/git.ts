/** The Git page's shapes, as `/api/git/**` sends them (`tendril-core/src/git/workspace`). */

export type RepoState = "clean" | "merging" | "rebasing" | "cherryPicking" | "reverting" | "bisecting";

export interface CommitBrief {
  hash: string;
  short: string;
  subject: string;
  author: string;
  /** Unix seconds. */
  at: number;
}

export interface RepoSummary {
  branch: string | null;
  detached: boolean;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  staged: number;
  unstaged: number;
  untracked: number;
  conflicted: number;
  state: RepoState;
  lastCommit: CommitBrief | null;
  localBranches: number;
  remoteBranches: number;
  stashes: number;
  worktrees: number;
  staleBranches: number;
}

export interface RepoCard {
  id: string;
  project: string;
  name: string;
  path: string;
  owner: string | null;
  summary: RepoSummary | null;
  error: string | null;
}

export type ChecksState = "passing" | "failing" | "pending" | "none";

export interface PrBrief {
  number: number;
  title: string;
  branch: string;
  base: string;
  state: "OPEN" | "MERGED" | "CLOSED" | string;
  url: string;
  draft: boolean;
  checks: ChecksState;
  author: string;
  review: string;
}

export interface RepoPrs {
  prs: PrBrief[];
  /** Why there are none to show (gh missing, not signed in, no GitHub remote), when that is the reason. */
  error: string | null;
}

export interface StatusEntry {
  path: string;
  origPath?: string;
  /** Staged side: M A D R C T, or "." */
  index: string;
  /** Unstaged side, same letters. */
  worktree: string;
  conflicted: boolean;
  untracked: boolean;
}

export interface RepoStatus {
  branch: string | null;
  detached: boolean;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  entries: StatusEntry[];
  state: RepoState;
}

export type RefKind = "local" | "remote" | "tag";

export interface RefInfo {
  name: string;
  fullRef: string;
  kind: RefKind;
  hash: string;
  upstream?: string;
  ahead: number;
  behind: number;
  gone: boolean;
  current: boolean;
  updated: string;
  subject: string;
  author: string;
  merged: boolean;
}

export interface StashInfo {
  index: number;
  message: string;
  at: number;
}

export interface WorktreeInfo {
  path: string;
  branch?: string;
  head: string;
  detached: boolean;
  bare: boolean;
  locked: boolean;
  prunable: boolean;
  main: boolean;
}

export interface RepoOverview {
  status: RepoStatus;
  refs: RefInfo[];
  stashes: StashInfo[];
  worktrees: WorktreeInfo[];
}

export interface BranchLink {
  kind: "mission" | "plan";
  id: string;
  title: string;
  state?: string;
}

export interface ActiveWork {
  id: string;
  title: string;
  state: string;
  branch?: string | null;
}

export interface RepoDetail {
  id: string;
  project: string;
  name: string;
  path: string;
  owner: string | null;
  overview: RepoOverview;
  /** Local branch name to the mission or plan it belongs to. */
  links: Record<string, BranchLink>;
  activeWork: ActiveWork[];
  /** Worktree path to the id that opens it as a repository. */
  worktreeIds: Record<string, string>;
}

export type GraphRefKind = "head" | "local" | "remote" | "tag";

export interface GraphRef {
  name: string;
  kind: GraphRefKind;
  isHead: boolean;
}

export type LinePart = "full" | "top" | "bottom";

export interface GraphLine {
  from: number;
  to: number;
  color: number;
  part: LinePart;
}

export interface GraphRow {
  hash: string;
  short: string;
  parents: string[];
  author: string;
  at: number;
  subject: string;
  refs: GraphRef[];
  lane: number;
  color: number;
  lines: GraphLine[];
}

export interface Graph {
  rows: GraphRow[];
  width: number;
  more: boolean;
}

export interface CommitFile {
  path: string;
  origPath?: string;
  status: string;
  additions: number;
  deletions: number;
  binary: boolean;
}

export interface CommitDetail {
  hash: string;
  short: string;
  parents: string[];
  author: string;
  email: string;
  at: number;
  subject: string;
  body: string;
  files: CommitFile[];
  additions: number;
  deletions: number;
}

export type DiffTarget =
  | { kind: "commit"; hash: string }
  | { kind: "staged" }
  | { kind: "unstaged" }
  | { kind: "untracked" };

export interface FileDiff {
  path: string;
  text: string;
  truncated: boolean;
  binary: boolean;
}

export interface PrPrefill {
  title: string;
  body: string;
  commits: string[];
}

export type PullMode = "ffOnly" | "rebase" | "merge";
export type MergeMode = "default" | "noFf" | "ffOnly";
export type ResetMode = "soft" | "mixed" | "hard";

/** One operation, tagged by `op`: what `POST /api/git/repos/:id/op` takes. */
export type RepoOp =
  | { op: "fetch"; prune?: boolean }
  | { op: "pull"; mode?: PullMode }
  | { op: "push"; branch?: string; setUpstream?: boolean; forceWithLease?: boolean }
  | { op: "checkout"; name: string }
  | { op: "checkoutRemote"; remoteBranch: string; localName?: string }
  | { op: "checkoutCommit"; hash: string }
  | { op: "createBranch"; name: string; from?: string; checkout?: boolean }
  | { op: "renameBranch"; from: string; to: string }
  | { op: "deleteBranch"; name: string; force?: boolean }
  | { op: "deleteRemoteBranch"; remote: string; name: string }
  | { op: "stage"; paths: string[] }
  | { op: "stageAll" }
  | { op: "unstage"; paths: string[] }
  | { op: "unstageAll" }
  | { op: "discard"; paths: string[] }
  | { op: "commit"; message?: string; amend?: boolean }
  | { op: "stash"; message?: string; includeUntracked?: boolean }
  | { op: "stashPop"; index: number }
  | { op: "stashApply"; index: number }
  | { op: "stashDrop"; index: number }
  | { op: "merge"; branch: string; mode?: MergeMode }
  | { op: "mergeAbort" }
  | { op: "rebase"; onto: string }
  | { op: "rebaseContinue" }
  | { op: "rebaseSkip" }
  | { op: "rebaseAbort" }
  | { op: "cherryPick"; hash: string }
  | { op: "cherryPickContinue" }
  | { op: "cherryPickAbort" }
  | { op: "revert"; hash: string }
  | { op: "revertContinue" }
  | { op: "revertAbort" }
  | { op: "createTag"; name: string; at?: string; message?: string }
  | { op: "deleteTag"; name: string }
  | { op: "reset"; to: string; mode: ResetMode }
  | { op: "worktreeAdd"; path: string; branch: string; create?: boolean }
  | { op: "worktreeRemove"; path: string; force?: boolean };

export interface OpOutcome {
  ok: boolean;
  message: string;
  conflicts: string[];
  state: RepoState;
}

export interface OpResult {
  outcome: OpOutcome;
  summary: RepoSummary;
}
