import React from "react";
import { ArrowDown, ArrowLeft, ArrowUp, Download, GitBranch, GitPullRequest, Layers, Plus, RefreshCw, Sparkles, Upload, X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { bridge } from "../../api/bridge";
import { describeBridgeError } from "../../types/api";
import type { CommitBrief, CommitDetail, GraphRow, MergeMode, PullMode, ResetMode } from "../../types/git";
import { GhostButton, Pill, PrimaryButton, Seg } from "../../components/page/kit";
import { BranchSidebar, type BranchAction } from "./BranchSidebar";
import { CommitGraph } from "./CommitGraph";
import { DiffPane } from "./DiffView";
import {
  DeleteBranchDialog,
  MergeDialog,
  NewBranchDialog,
  PrDialog,
  PushDialog,
  RenameBranchDialog,
  ResetDialog,
  StashDialog,
  TagDialog,
} from "./GitDialogs";
import { CommitPanel, WorkingPanel, type OpenFile } from "./Panels";
import { ago, ConfirmDialog, useContextMenu } from "./gitUi";
import { useRepoWorkspace } from "./useRepoWorkspace";

type Dialog =
  | { k: "newBranch"; from: { label: string; rev: string | undefined } }
  | { k: "rename"; name: string }
  | { k: "merge"; branch: string }
  | { k: "delete"; name: string; merged: boolean }
  | { k: "tag"; at: string; label: string }
  | { k: "stash" }
  | { k: "reset"; hash: string; short: string; subject: string }
  | { k: "push" }
  | { k: "pr"; head: string }
  | { k: "confirm"; title: string; body: React.ReactNode; confirmLabel: string; danger?: boolean; run: () => void };

/** What to call the operation a state belongs to, in a sentence. */
const OPERATION_NAME: Record<string, string> = {
  merging: "merge",
  rebasing: "rebase",
  cherryPicking: "cherry-pick",
  reverting: "revert",
  bisecting: "bisect",
};

const STATE_LABEL: Record<string, string> = {
  merging: "A merge is in progress",
  rebasing: "A rebase is in progress",
  cherryPicking: "A cherry-pick is in progress",
  reverting: "A revert is in progress",
  bisecting: "A bisect is in progress",
};

/** One file's history, in the centre pane. */
const FileHistory: React.FC<{ repoId: string; path: string; onClose: () => void; onSelect: (hash: string) => void }> = ({ repoId, path, onClose, onSelect }) => {
  const [commits, setCommits] = React.useState<CommitBrief[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    let live = true;
    bridge
      .gitHistory(repoId, path)
      .then((c) => live && setCommits(c))
      .catch((e) => live && setError(describeBridgeError(e)));
    return () => {
      live = false;
    };
  }, [repoId, path]);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-3 border-b border-border/70 px-4 py-2.5">
        <GhostButton size="sm" onClick={onClose}>
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Graph
        </GhostButton>
        <span className="min-w-0 truncate font-mono text-[12.5px] text-foreground">History of {path}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {error && <div className="p-4 text-[12.5px] text-destructive">{error}</div>}
        {!commits && !error && <div className="p-4 text-[12.5px] text-muted-foreground">Loading…</div>}
        {commits?.map((c) => (
          <button key={c.hash} type="button" onClick={() => onSelect(c.hash)} className="flex w-full items-center gap-3 border-b border-border/40 px-4 py-2 text-left hover:bg-secondary/50">
            <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground">{c.subject}</span>
            <span className="shrink-0 text-[11.5px] text-muted-foreground">{c.author}</span>
            <span className="w-[64px] shrink-0 text-right font-mono text-[10.5px] text-muted-foreground">{ago(c.at).replace(" ago", "")}</span>
            <span className="w-[56px] shrink-0 font-mono text-[10.5px] text-muted-foreground">{c.short}</span>
          </button>
        ))}
      </div>
    </div>
  );
};

/** A repository, GitKraken-lite: branches on the left, the graph in the middle, the selected commit or working changes on the right. */
export const RepoWorkspace: React.FC<{
  repoId: string;
  onBack: () => void;
  onOpenRepo: (id: string) => void;
  /** Hands a problem to the project's manager as a drafted message. */
  onAskManager: (project: string, text: string) => void;
}> = ({ repoId, onBack, onOpenRepo, onAskManager }) => {
  const ws = useRepoWorkspace(repoId);
  const { detail, graph } = ws;
  const [selected, setSelected] = React.useState<string | null>(null);
  const [opened, setOpened] = React.useState<OpenFile | null>(null);
  const [history, setHistory] = React.useState<string | null>(null);
  const [dialog, setDialog] = React.useState<Dialog | null>(null);
  const [pullMode, setPullMode] = React.useState<PullMode>("ffOnly");
  const [pane, setPane] = React.useState<"branches" | "graph" | "details">("graph");
  const [hideWarning, setHideWarning] = React.useState(false);
  const menu = useContextMenu();
  const seeded = React.useRef<string | null>(null);

  const status = detail?.overview.status;
  const dirtyCount = status?.entries.length ?? 0;
  const version = React.useMemo(
    () => (detail ? `${status?.head}|${status?.entries.map((e) => `${e.path}${e.index}${e.worktree}`).join(",")}` : ""),
    [detail, status],
  );

  // Start on what needs doing: the working changes if there are any, otherwise where HEAD is.
  React.useEffect(() => {
    if (!detail || seeded.current === repoId) return;
    seeded.current = repoId;
    setSelected(dirtyCount > 0 ? "wip" : (status?.head ?? null));
  }, [detail, repoId, dirtyCount, status?.head]);

  // An operation stopped on conflicts: the conflicting files are what to look at now.
  React.useEffect(() => {
    if (ws.notice?.kind === "conflict") {
      setSelected("wip");
      setPane("details");
    }
  }, [ws.notice]);

  // The tree became clean while looking at it: show the newest commit instead of an empty list.
  React.useEffect(() => {
    if (selected === "wip" && detail && dirtyCount === 0) setSelected(status?.head ?? null);
  }, [selected, detail, dirtyCount, status?.head]);

  React.useEffect(() => {
    setOpened(null);
    setHistory(null);
    seeded.current = null;
    setSelected(null);
  }, [repoId]);

  const select = (s: string) => {
    setSelected(s);
    setPane("details");
  };

  if (ws.error && !detail) {
    return (
      <div className="flex flex-1 flex-col items-start gap-3 p-6">
        <GhostButton onClick={onBack}>
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Git
        </GhostButton>
        <div className="text-[13px] text-destructive">{ws.error}</div>
      </div>
    );
  }
  if (!detail || !graph || !status) {
    return (
      <div className="flex flex-1 flex-col gap-3 p-6">
        <GhostButton onClick={onBack} className="w-fit">
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Git
        </GhostButton>
        <div className="text-[13px] text-muted-foreground">Reading the repository…</div>
      </div>
    );
  }

  const { overview } = detail;
  const current = status.branch;
  const busy = ws.busy !== null;
  const locals = overview.refs.filter((r) => r.kind === "local");
  const localByName = (n: string) => locals.find((r) => r.name === n);
  const bases = Array.from(new Set(overview.refs.filter((r) => r.kind !== "tag").map((r) => (r.kind === "remote" ? r.name.split("/").slice(1).join("/") : r.name))));
  const defaultBase = bases.includes("main") ? "main" : bases.includes("master") ? "master" : (bases[0] ?? "main");
  const conflicted = status.entries.filter((e) => e.conflicted).map((e) => e.path);
  const close = () => setDialog(null);
  const confirm = (title: string, body: React.ReactNode, confirmLabel: string, run: () => void, danger = false) =>
    setDialog({ k: "confirm", title, body, confirmLabel, danger, run });

  const askManager = (what: string) =>
    onAskManager(
      detail.project,
      `In the ${detail.name} repository (${detail.path}), ${what}${conflicted.length ? ` Conflicting files: ${conflicted.join(", ")}.` : ""} Please sort it out with a worker task (leave my branch history intact, and don't push), then tell me what you did.`,
    );

  /* ---- actions ----------------------------------------------------------------------------- */

  const onBranchAction = (a: BranchAction) => {
    switch (a.type) {
      case "checkout":
        return void ws.run({ op: "checkout", name: a.name }, `Switching to ${a.name}`);
      case "checkoutRemote":
        return void ws.run({ op: "checkoutRemote", remoteBranch: a.name }, `Checking out ${a.name}`);
      case "merge":
        return setDialog({ k: "merge", branch: a.name });
      case "rebase":
        return confirm(
          `Rebase ${current} onto ${a.name}?`,
          "This replays your commits on top of that branch, which rewrites them. If you have already pushed this branch you will need a force push afterwards. If it stops on conflicts you can abort.",
          "Rebase",
          () => void ws.run({ op: "rebase", onto: a.name }, `Rebasing onto ${a.name}`),
        );
      case "push": {
        const ref = localByName(a.name);
        if (a.name === current) return setDialog({ k: "push" });
        return void ws.run({ op: "push", branch: a.name, setUpstream: !ref?.upstream }, `Pushing ${a.name}`);
      }
      case "delete":
        return setDialog({ k: "delete", name: a.name, merged: a.merged });
      case "rename":
        return setDialog({ k: "rename", name: a.name });
      case "deleteRemote": {
        return confirm(`Delete ${a.remote}/${a.name} on the remote?`, "This removes the branch from the remote for everyone. Your local copy, if you have one, is not touched.", "Delete on remote", () => void ws.run({ op: "deleteRemoteBranch", remote: a.remote, name: a.name }, `Deleting ${a.remote}/${a.name}`), true);
      }
      case "pr":
        return setDialog({ k: "pr", head: a.name });
      case "branchFrom":
        return setDialog({ k: "newBranch", from: { label: a.hash.slice(0, 7), rev: a.hash } });
      case "tagAt":
        return setDialog({ k: "tag", at: a.hash, label: a.hash.slice(0, 7) });
      case "deleteTag":
        return confirm(`Delete the tag ${a.name}?`, "Only the local tag is deleted.", "Delete tag", () => void ws.run({ op: "deleteTag", name: a.name }, `Deleting ${a.name}`), true);
      case "checkoutCommit":
        return confirm("Check out this commit?", "You will be on a detached HEAD: commits made there belong to no branch until you create one.", "Check out", () => void ws.run({ op: "checkoutCommit", hash: a.hash }, "Checking out the commit"));
      case "stash":
        if (a.op === "drop") return confirm("Drop this stash?", "The stashed changes are deleted and cannot be brought back.", "Drop stash", () => void ws.run({ op: "stashDrop", index: a.index }, "Dropping the stash"), true);
        return void ws.run(a.op === "pop" ? { op: "stashPop", index: a.index } : { op: "stashApply", index: a.index }, a.op === "pop" ? "Popping the stash" : "Applying the stash");
      case "worktreeRemove":
        return confirm("Remove this worktree?", `${a.path}\n\nThe folder is deleted. Its branch stays.`, "Remove worktree", () => void ws.run({ op: "worktreeRemove", path: a.path }, "Removing the worktree"), true);
    }
  };

  const onCommitAction = (a: "checkout" | "branch" | "cherryPick" | "revert" | "tag" | "reset", c: CommitDetail) => {
    if (a === "checkout") return onBranchAction({ type: "checkoutCommit", hash: c.hash });
    if (a === "branch") return onBranchAction({ type: "branchFrom", hash: c.hash });
    if (a === "tag") return onBranchAction({ type: "tagAt", hash: c.hash });
    if (a === "cherryPick") return void ws.run({ op: "cherryPick", hash: c.hash }, "Cherry-picking");
    if (a === "revert") return void ws.run({ op: "revert", hash: c.hash }, "Reverting");
    return setDialog({ k: "reset", hash: c.hash, short: c.short, subject: c.subject });
  };

  const onGraphContext = (row: GraphRow, e: React.MouseEvent) => {
    const branches = row.refs.filter((r) => r.kind === "local" && !r.isHead);
    menu.open(e, [
      ...branches.map((b) => ({ label: `Check out ${b.name}`, onClick: () => onBranchAction({ type: "checkout", name: b.name }), disabled: busy })),
      { label: "Check out this commit", onClick: () => onBranchAction({ type: "checkoutCommit", hash: row.hash }), disabled: busy, separator: branches.length > 0 },
      { label: "Create branch here…", onClick: () => onBranchAction({ type: "branchFrom", hash: row.hash }), disabled: busy },
      { label: "Create tag here…", onClick: () => onBranchAction({ type: "tagAt", hash: row.hash }), disabled: busy },
      { label: "Cherry-pick", onClick: () => void ws.run({ op: "cherryPick", hash: row.hash }, "Cherry-picking"), disabled: busy, separator: true },
      { label: "Revert", onClick: () => void ws.run({ op: "revert", hash: row.hash }, "Reverting"), disabled: busy },
      { label: "Reset current branch to here…", onClick: () => setDialog({ k: "reset", hash: row.hash, short: row.short, subject: row.subject }), disabled: busy, danger: true },
      { label: "Copy hash", onClick: () => void navigator.clipboard?.writeText(row.hash), separator: true },
    ]);
  };

  const stateBanner = status.state !== "clean" && (
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2.5 text-[12.5px]">
      <span className="font-medium text-warning">{STATE_LABEL[status.state] ?? "An operation is in progress"}</span>
      <span className="text-muted-foreground">{conflicted.length > 0 ? `${conflicted.length} file${conflicted.length === 1 ? "" : "s"} in conflict.` : "No conflicts left: you can finish it."}</span>
      <span className="ml-auto flex flex-wrap gap-1.5">
        {status.state === "merging" && <GhostButton size="sm" disabled={busy} onClick={() => confirm("Abort the merge?", "Your branch goes back to how it was before the merge. Nothing is lost.", "Abort merge", () => void ws.run({ op: "mergeAbort" }, "Aborting the merge"))}>Abort merge</GhostButton>}
        {status.state === "rebasing" && (
          <>
            <GhostButton size="sm" disabled={busy || conflicted.length > 0} onClick={() => void ws.run({ op: "rebaseContinue" }, "Continuing the rebase")}>Continue</GhostButton>
            <GhostButton size="sm" disabled={busy} onClick={() => void ws.run({ op: "rebaseSkip" }, "Skipping")}>Skip commit</GhostButton>
            <GhostButton size="sm" disabled={busy} onClick={() => confirm("Abort the rebase?", "Your branch goes back to how it was before the rebase.", "Abort rebase", () => void ws.run({ op: "rebaseAbort" }, "Aborting the rebase"))}>Abort rebase</GhostButton>
          </>
        )}
        {status.state === "cherryPicking" && (
          <>
            <GhostButton size="sm" disabled={busy || conflicted.length > 0} onClick={() => void ws.run({ op: "cherryPickContinue" }, "Continuing")}>Continue</GhostButton>
            <GhostButton size="sm" disabled={busy} onClick={() => void ws.run({ op: "cherryPickAbort" }, "Aborting")}>Abort</GhostButton>
          </>
        )}
        {status.state === "reverting" && (
          <>
            <GhostButton size="sm" disabled={busy || conflicted.length > 0} onClick={() => void ws.run({ op: "revertContinue" }, "Continuing")}>Continue</GhostButton>
            <GhostButton size="sm" disabled={busy} onClick={() => void ws.run({ op: "revertAbort" }, "Aborting")}>Abort</GhostButton>
          </>
        )}
        {conflicted.length > 0 && (
          <PrimaryButton size="sm" onClick={() => askManager(`a ${OPERATION_NAME[status.state] ?? "git operation"} stopped on conflicts.`)}>
            <Sparkles className="size-3.5" aria-hidden="true" />
            Ask the manager to resolve
          </PrimaryButton>
        )}
      </span>
    </div>
  );

  const notice = ws.notice && (
    <div
      className={cn(
        "flex shrink-0 items-start gap-3 border-b px-4 py-2 text-[12.5px]",
        ws.notice.kind === "ok" ? "border-primary/25 bg-primary/8 text-foreground" : ws.notice.kind === "conflict" ? "border-warning/30 bg-warning/10 text-foreground" : "border-destructive/30 bg-destructive/10 text-foreground",
      )}
      role="status"
    >
      <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">
        <b className="font-medium">{ws.notice.op ? `${ws.notice.op}: ` : ""}</b>
        {ws.notice.message}
        {ws.notice.conflicts.length > 0 && <span className="block font-mono text-[11px] text-muted-foreground">{ws.notice.conflicts.join(", ")}</span>}
      </span>
      <button type="button" aria-label="Dismiss" onClick={ws.dismissNotice} className="shrink-0 text-muted-foreground hover:text-foreground">
        <X className="size-3.5" aria-hidden="true" />
      </button>
    </div>
  );

  const toolbarButton = (label: string, icon: React.ReactNode, onClick: () => void, disabled = false, badge?: React.ReactNode) => (
    <GhostButton size="sm" disabled={busy || disabled} onClick={onClick} title={label}>
      {icon}
      <span className="hidden @[1180px]:inline">{label}</span>
      {badge}
    </GhostButton>
  );

  /* ---- layout ------------------------------------------------------------------------------ */

  const centre = history ? (
    <FileHistory repoId={repoId} path={history} onClose={() => setHistory(null)} onSelect={(h) => (setHistory(null), setOpened(null), select(h))} />
  ) : opened ? (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-3 border-b border-border/70 px-4 py-2.5">
        <GhostButton size="sm" onClick={() => setOpened(null)}>
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Graph
        </GhostButton>
        <span className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-foreground">{opened.label}</span>
        <GhostButton size="sm" onClick={() => setHistory(opened.path)}>History</GhostButton>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <DiffPane repoId={repoId} target={opened.target} path={opened.path} origPath={opened.origPath} version={version} />
      </div>
    </div>
  ) : (
    <CommitGraph
      graph={graph}
      selected={selected}
      wip={dirtyCount > 0 ? { count: dirtyCount, conflicted: conflicted.length } : null}
      onSelect={select}
      onContext={onGraphContext}
      onLoadMore={ws.loadMore}
    />
  );

  const right =
    selected === "wip" ? (
      <WorkingPanel
        entries={status.entries}
        busy={busy}
        merging={status.state === "merging"}
        opened={opened}
        onOpenFile={(f) => (setOpened(f), setPane("graph"))}
        onStage={(paths) => void ws.run({ op: "stage", paths }, "Staging")}
        onUnstage={(paths) => void ws.run({ op: "unstage", paths }, "Unstaging")}
        onDiscard={(paths) =>
          confirm(
            paths.length === 1 ? `Discard changes to ${paths[0]}?` : `Discard changes to ${paths.length} files?`,
            "The changes are thrown away and cannot be recovered. A new (untracked) file is deleted.",
            "Discard",
            () => void ws.run({ op: "discard", paths }, "Discarding"),
            true,
          )
        }
        onStageAll={() => void ws.run({ op: "stageAll" }, "Staging everything")}
        onUnstageAll={() => void ws.run({ op: "unstageAll" }, "Unstaging everything")}
        onCommit={(message, amend) => ws.run({ op: "commit", message, amend }, amend ? "Amending" : "Committing")}
      />
    ) : selected ? (
      <CommitPanel
        repoId={repoId}
        hash={selected}
        version={status.head}
        busy={busy}
        opened={opened}
        onOpenFile={(f) => (setOpened(f), setPane("graph"))}
        onHistory={(p) => (setHistory(p), setPane("graph"))}
        onSelectCommit={select}
        onAction={onCommitAction}
      />
    ) : (
      <div className="p-6 text-[12.5px] text-muted-foreground">Select a commit.</div>
    );

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="repo-workspace">
      <header className="@container flex shrink-0 flex-wrap items-center gap-2 border-b border-border/70 px-4 py-2.5">
        <GhostButton size="sm" onClick={onBack} aria-label="Back to repositories">
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Git
        </GhostButton>
        <div className="flex min-w-0 flex-col leading-tight">
          <span className="truncate text-[14px] font-semibold text-foreground">{detail.name}</span>
          <span className="truncate font-mono text-[10.5px] text-muted-foreground">
            {detail.project}
            {detail.owner ? ` · ${detail.owner}` : ""}
          </span>
        </div>
        <span className="inline-flex min-w-0 items-center gap-1.5 rounded-md bg-muted px-2 py-1 font-mono text-[12px] text-foreground">
          <GitBranch className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="truncate">{status.detached ? `detached @ ${status.head?.slice(0, 7)}` : (current ?? "no commits")}</span>
        </span>
        {status.ahead > 0 && <Pill tone="info"><ArrowUp className="size-3" aria-hidden="true" /> {status.ahead}</Pill>}
        {status.behind > 0 && <Pill tone="warn"><ArrowDown className="size-3" aria-hidden="true" /> {status.behind}</Pill>}
        {current && !status.upstream && !status.detached && <Pill>not on the remote</Pill>}
        {ws.busy && <Pill tone="info" dot live>{ws.busy}…</Pill>}

        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          {toolbarButton("Fetch", <RefreshCw className="size-3.5" aria-hidden="true" />, () => void ws.run({ op: "fetch", prune: true }, "Fetching"))}
          <span className="inline-flex items-center gap-1">
            {toolbarButton("Pull", <Download className="size-3.5" aria-hidden="true" />, () => void ws.run({ op: "pull", mode: pullMode }, "Pulling"), status.detached || !status.upstream, status.behind > 0 ? <span className="font-mono text-[10.5px] text-warning">{status.behind}</span> : undefined)}
            <select
              aria-label="How to pull"
              value={pullMode}
              onChange={(e) => setPullMode(e.target.value as PullMode)}
              className="h-7 rounded-lg border border-input bg-muted px-1.5 text-[11.5px] text-foreground outline-none"
              title="How to bring in remote commits"
            >
              <option value="ffOnly">Fast-forward only</option>
              <option value="rebase">Rebase</option>
              <option value="merge">Merge</option>
            </select>
          </span>
          {toolbarButton("Push", <Upload className="size-3.5" aria-hidden="true" />, () => setDialog({ k: "push" }), status.detached || !current, status.ahead > 0 ? <span className="font-mono text-[10.5px] text-info">{status.ahead}</span> : undefined)}
          {toolbarButton("Branch", <Plus className="size-3.5" aria-hidden="true" />, () => setDialog({ k: "newBranch", from: { label: current ?? "HEAD", rev: undefined } }), !status.head)}
          {toolbarButton("Stash", <Layers className="size-3.5" aria-hidden="true" />, () => setDialog({ k: "stash" }), dirtyCount === 0)}
          {toolbarButton("Pull request", <GitPullRequest className="size-3.5" aria-hidden="true" />, () => current && setDialog({ k: "pr", head: current }), !current || status.detached)}
          <GhostButton size="sm" onClick={() => void ws.refresh()} aria-label="Refresh" title="Refresh">
            <RefreshCw className="size-3.5" aria-hidden="true" />
          </GhostButton>
        </div>
      </header>

      {stateBanner}
      {notice}
      {detail.activeWork.length > 0 && !hideWarning && (
        <div className="flex shrink-0 items-center gap-3 border-b border-info/25 bg-info/8 px-4 py-2 text-[12.5px]">
          <span className="text-foreground">
            {detail.activeWork.length === 1 ? `A mission is running in ${detail.project}: “${detail.activeWork[0].title}”.` : `${detail.activeWork.length} missions are running in ${detail.project}.`}{" "}
            <span className="text-muted-foreground">Changes you make here can collide with its work.</span>
          </span>
          <button type="button" aria-label="Dismiss" onClick={() => setHideWarning(true)} className="ml-auto text-muted-foreground hover:text-foreground">
            <X className="size-3.5" aria-hidden="true" />
          </button>
        </div>
      )}

      <Seg<"branches" | "graph" | "details">
        className="m-2 w-fit lg:hidden"
        label="Pane"
        value={pane}
        onChange={setPane}
        options={[
          { value: "branches", label: "Branches" },
          { value: "graph", label: "Graph" },
          { value: "details", label: "Details" },
        ]}
      />

      <div className="flex min-h-0 flex-1">
        <div className={cn("min-h-0 w-full flex-col lg:flex lg:w-[236px] lg:shrink-0", pane === "branches" ? "flex" : "hidden")}>
          <BranchSidebar
            detail={detail}
            prs={ws.prs}
            filter={ws.branchFilter}
            selectedHash={selected}
            busy={busy}
            onSelectRef={(hash) => (setOpened(null), setHistory(null), select(hash))}
            onFilter={ws.setBranchFilter}
            onAction={onBranchAction}
            onOpenWorktree={onOpenRepo}
          />
        </div>
        <div className={cn("min-h-0 min-w-0 flex-1 flex-col lg:flex", pane === "graph" ? "flex" : "hidden")}>{centre}</div>
        <div className={cn("min-h-0 w-full flex-col border-l border-border/70 lg:flex lg:w-[372px] lg:shrink-0", pane === "details" ? "flex" : "hidden")}>{right}</div>
      </div>

      {menu.element}

      {dialog?.k === "newBranch" && (
        <NewBranchDialog
          from={dialog.from}
          onClose={close}
          onSubmit={(name, checkout) => void ws.run({ op: "createBranch", name, from: dialog.from.rev, checkout }, `Creating ${name}`)}
        />
      )}
      {dialog?.k === "rename" && <RenameBranchDialog name={dialog.name} onClose={close} onSubmit={(to) => void ws.run({ op: "renameBranch", from: dialog.name, to }, "Renaming")} />}
      {dialog?.k === "merge" && current && (
        <MergeDialog current={current} branch={dialog.branch} onClose={close} onSubmit={(mode: MergeMode) => void ws.run({ op: "merge", branch: dialog.branch, mode }, `Merging ${dialog.branch}`)} />
      )}
      {dialog?.k === "delete" && (
        <DeleteBranchDialog name={dialog.name} merged={dialog.merged} onClose={close} onSubmit={(force) => void ws.run({ op: "deleteBranch", name: dialog.name, force }, `Deleting ${dialog.name}`)} />
      )}
      {dialog?.k === "tag" && <TagDialog at={dialog.label} onClose={close} onSubmit={(name, message) => void ws.run({ op: "createTag", name, at: dialog.at, message: message || undefined }, `Tagging ${name}`)} />}
      {dialog?.k === "stash" && <StashDialog onClose={close} onSubmit={(message, includeUntracked) => void ws.run({ op: "stash", message: message || undefined, includeUntracked }, "Stashing")} />}
      {dialog?.k === "reset" && (
        <ResetDialog short={dialog.short} subject={dialog.subject} onClose={close} onSubmit={(mode: ResetMode) => void ws.run({ op: "reset", to: dialog.hash, mode }, "Resetting")} />
      )}
      {dialog?.k === "push" && current && (
        <PushDialog
          branch={current}
          hasUpstream={!!status.upstream}
          onClose={close}
          onSubmit={({ setUpstream, forceWithLease }) => void ws.run({ op: "push", setUpstream, forceWithLease }, `Pushing ${current}`)}
        />
      )}
      {dialog?.k === "pr" && <PrDialog repoId={repoId} head={dialog.head} bases={bases} defaultBase={defaultBase} onClose={close} onCreated={() => void ws.loadPrs()} />}
      {dialog?.k === "confirm" && <ConfirmDialog title={dialog.title} body={<span className="whitespace-pre-wrap">{dialog.body}</span>} confirmLabel={dialog.confirmLabel} danger={dialog.danger} onConfirm={dialog.run} onClose={close} />}
    </div>
  );
};
