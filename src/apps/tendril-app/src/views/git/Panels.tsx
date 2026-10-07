import React from "react";
import { Check, Copy, FileText, History, Minus, Plus, Undo2 } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { bridge } from "../../api/bridge";
import { describeBridgeError } from "../../types/api";
import type { CommitDetail, CommitFile, DiffTarget, StatusEntry } from "../../types/git";
import { GhostButton, Pill, PrimaryButton } from "../../components/page/kit";
import { ago, textareaClass } from "./gitUi";

/** What the centre pane should show a diff of. */
export interface OpenFile {
  target: DiffTarget;
  path: string;
  origPath?: string;
  label: string;
}

const STATUS_COLOR: Record<string, string> = {
  A: "text-success",
  M: "text-warning",
  D: "text-destructive",
  R: "text-info",
  C: "text-info",
  T: "text-violet",
  "?": "text-muted-foreground",
  U: "text-destructive",
};

const StatusLetter: React.FC<{ letter: string }> = ({ letter }) => (
  <span className={cn("w-4 shrink-0 text-center font-mono text-[11px] font-semibold", STATUS_COLOR[letter] ?? "text-muted-foreground")}>{letter === "." ? "" : letter}</span>
);

const baseName = (path: string) => path.split("/").pop() ?? path;
const dirName = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/") + 1) : "");

const FileLabel: React.FC<{ path: string; orig?: string }> = ({ path, orig }) => (
  <span className="flex min-w-0 flex-1 items-baseline gap-1.5" title={orig ? `${orig} → ${path}` : path}>
    <span className="truncate text-[12.5px] text-foreground">{baseName(path)}</span>
    <span className="truncate font-mono text-[10.5px] text-muted-foreground">{orig ? `from ${baseName(orig)}` : dirName(path)}</span>
  </span>
);

export const CommitPanel: React.FC<{
  repoId: string;
  hash: string;
  version: unknown;
  busy: boolean;
  opened: OpenFile | null;
  onOpenFile: (f: OpenFile) => void;
  onHistory: (path: string) => void;
  onSelectCommit: (hash: string) => void;
  onAction: (a: "checkout" | "branch" | "cherryPick" | "revert" | "tag" | "reset", detail: CommitDetail) => void;
}> = ({ repoId, hash, version, busy, opened, onOpenFile, onHistory, onSelectCommit, onAction }) => {
  const [detail, setDetail] = React.useState<CommitDetail | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState(false);

  React.useEffect(() => {
    let live = true;
    setError(null);
    bridge
      .gitCommit(repoId, hash)
      .then((d) => live && setDetail(d))
      .catch((e) => live && setError(describeBridgeError(e)));
    return () => {
      live = false;
    };
  }, [repoId, hash, version]);

  if (error) return <div className="p-4 text-[12.5px] text-destructive">{error}</div>;
  if (!detail || detail.hash !== hash) return <div className="p-4 text-[12.5px] text-muted-foreground">Loading…</div>;

  const copy = () => {
    void navigator.clipboard?.writeText(detail.hash).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className="flex flex-col gap-2 border-b border-border/70 p-4">
        <div className="text-[14px] font-semibold leading-snug text-foreground">{detail.subject}</div>
        {detail.body && <p className="m-0 whitespace-pre-wrap text-[12.5px] leading-relaxed text-muted-foreground">{detail.body}</p>}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-muted-foreground">
          <span>{detail.author}</span>
          <span>{ago(detail.at)}</span>
          <button type="button" onClick={copy} className="inline-flex items-center gap-1 hover:text-foreground" title="Copy the full hash">
            {copied ? <Check className="size-3 text-success" aria-hidden="true" /> : <Copy className="size-3" aria-hidden="true" />}
            {detail.short}
          </button>
          {detail.parents.map((p) => (
            <button key={p} type="button" onClick={() => onSelectCommit(p)} className="text-info hover:underline" title="Go to the parent commit">
              parent {p.slice(0, 7)}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap gap-1.5 pt-1">
          <GhostButton size="sm" disabled={busy} onClick={() => onAction("checkout", detail)}>Check out</GhostButton>
          <GhostButton size="sm" disabled={busy} onClick={() => onAction("branch", detail)}>Branch here</GhostButton>
          <GhostButton size="sm" disabled={busy} onClick={() => onAction("cherryPick", detail)}>Cherry-pick</GhostButton>
          <GhostButton size="sm" disabled={busy} onClick={() => onAction("revert", detail)}>Revert</GhostButton>
          <GhostButton size="sm" disabled={busy} onClick={() => onAction("tag", detail)}>Tag</GhostButton>
          <GhostButton size="sm" disabled={busy} onClick={() => onAction("reset", detail)}>Reset to here…</GhostButton>
        </div>
      </div>

      <div className="flex items-center gap-2 px-4 pb-1 pt-3 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
        {detail.files.length} file{detail.files.length === 1 ? "" : "s"}
        <span className="text-success">+{detail.additions}</span>
        <span className="text-destructive">−{detail.deletions}</span>
        {detail.parents.length > 1 && <span className="normal-case">· against the first parent</span>}
      </div>
      <div className="flex flex-col pb-4">
        {detail.files.map((f: CommitFile) => {
          const active = opened?.path === f.path && opened.target.kind === "commit" && opened.target.hash === detail.hash;
          return (
            <div key={f.path} className={cn("group mx-2 flex items-center gap-2 rounded-md px-2 py-1 hover:bg-secondary/60", active && "bg-secondary")}>
              <button
                type="button"
                className="flex min-w-0 flex-1 items-center gap-2 text-left"
                onClick={() => onOpenFile({ target: { kind: "commit", hash: detail.hash }, path: f.path, origPath: f.origPath, label: f.path })}
              >
                <StatusLetter letter={f.status} />
                <FileLabel path={f.path} orig={f.origPath} />
                {f.binary ? (
                  <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">binary</span>
                ) : (
                  <span className="shrink-0 font-mono text-[10.5px]">
                    <span className="text-success">+{f.additions}</span> <span className="text-destructive">−{f.deletions}</span>
                  </span>
                )}
              </button>
              <button type="button" aria-label={`History of ${f.path}`} title="History of this file" onClick={() => onHistory(f.path)} className="hidden shrink-0 text-muted-foreground hover:text-foreground group-hover:block">
                <History className="size-3.5" aria-hidden="true" />
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const FileRow: React.FC<{
  entry: StatusEntry;
  letter: string;
  active: boolean;
  busy: boolean;
  onOpen: () => void;
  actions: { label: string; icon: React.ReactNode; onClick: () => void; danger?: boolean }[];
}> = ({ entry, letter, active, busy, onOpen, actions }) => (
  <div className={cn("group mx-2 flex items-center gap-2 rounded-md px-2 py-1 hover:bg-secondary/60", active && "bg-secondary")}>
    <button type="button" onClick={onOpen} className="flex min-w-0 flex-1 items-center gap-2 text-left">
      <StatusLetter letter={letter} />
      <FileLabel path={entry.path} orig={entry.origPath} />
    </button>
    <span className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
      {actions.map((a) => (
        <button
          key={a.label}
          type="button"
          title={a.label}
          aria-label={`${a.label} ${entry.path}`}
          disabled={busy}
          onClick={a.onClick}
          className={cn("rounded p-1 hover:bg-background disabled:opacity-40", a.danger ? "text-destructive" : "text-muted-foreground hover:text-foreground")}
        >
          {a.icon}
        </button>
      ))}
    </span>
  </div>
);

const Group: React.FC<{ title: string; count: number; action?: React.ReactNode; children: React.ReactNode }> = ({ title, count, action, children }) =>
  count === 0 ? null : (
    <section className="flex flex-col pb-2">
      <div className="flex items-center gap-2 px-4 py-1.5 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
        {title}
        <span className="rounded-full bg-secondary px-1.5 text-[10px]">{count}</span>
        <span className="ml-auto normal-case tracking-normal">{action}</span>
      </div>
      {children}
    </section>
  );

export const WorkingPanel: React.FC<{
  entries: StatusEntry[];
  busy: boolean;
  merging: boolean;
  opened: OpenFile | null;
  onOpenFile: (f: OpenFile) => void;
  onStage: (paths: string[]) => void;
  onUnstage: (paths: string[]) => void;
  onDiscard: (paths: string[]) => void;
  onStageAll: () => void;
  onUnstageAll: () => void;
  onCommit: (message: string, amend: boolean) => Promise<boolean>;
}> = ({ entries, busy, merging, opened, onOpenFile, onStage, onUnstage, onDiscard, onStageAll, onUnstageAll, onCommit }) => {
  const [message, setMessage] = React.useState("");
  const [amend, setAmend] = React.useState(false);

  const conflicted = entries.filter((e) => e.conflicted);
  const staged = entries.filter((e) => !e.conflicted && !e.untracked && e.index !== ".");
  const unstaged = entries.filter((e) => !e.conflicted && !e.untracked && e.worktree !== ".");
  const untracked = entries.filter((e) => e.untracked);
  const isOpen = (path: string, kind: DiffTarget["kind"]) => opened?.path === path && opened.target.kind === kind;
  const canCommit = !busy && conflicted.length === 0 && (merging || amend || staged.length > 0) && (message.trim() !== "" || amend);

  const submit = async () => {
    if (!canCommit) return;
    if (await onCommit(message, amend)) {
      setMessage("");
      setAmend(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border/70 px-4 py-3">
        <span className="mr-auto text-[13px] font-semibold text-foreground">Working changes</span>
        <GhostButton size="sm" disabled={busy || unstaged.length + untracked.length === 0} onClick={onStageAll}>Stage all</GhostButton>
        <GhostButton size="sm" disabled={busy || staged.length === 0} onClick={onUnstageAll}>Unstage all</GhostButton>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        {entries.length === 0 && <div className="px-4 py-8 text-center text-[12.5px] text-muted-foreground">Nothing has changed. The working tree is clean.</div>}

        <Group title="Conflicts" count={conflicted.length}>
          {conflicted.map((e) => (
            <FileRow
              key={e.path}
              entry={e}
              letter="U"
              active={isOpen(e.path, "unstaged")}
              busy={busy}
              onOpen={() => onOpenFile({ target: { kind: "unstaged" }, path: e.path, label: `${e.path} (conflict)` })}
              actions={[{ label: "Mark resolved (stage)", icon: <Plus className="size-3.5" aria-hidden="true" />, onClick: () => onStage([e.path]) }]}
            />
          ))}
        </Group>

        <Group title="Staged" count={staged.length} action={<button type="button" className="text-info hover:underline" onClick={onUnstageAll}>unstage all</button>}>
          {staged.map((e) => (
            <FileRow
              key={`s-${e.path}`}
              entry={e}
              letter={e.index}
              active={isOpen(e.path, "staged")}
              busy={busy}
              onOpen={() => onOpenFile({ target: { kind: "staged" }, path: e.path, origPath: e.origPath, label: `${e.path} (staged)` })}
              actions={[{ label: "Unstage", icon: <Minus className="size-3.5" aria-hidden="true" />, onClick: () => onUnstage([e.path]) }]}
            />
          ))}
        </Group>

        <Group title="Changes" count={unstaged.length} action={<button type="button" className="text-info hover:underline" onClick={onStageAll}>stage all</button>}>
          {unstaged.map((e) => (
            <FileRow
              key={`u-${e.path}`}
              entry={e}
              letter={e.worktree}
              active={isOpen(e.path, "unstaged")}
              busy={busy}
              onOpen={() => onOpenFile({ target: { kind: "unstaged" }, path: e.path, label: `${e.path} (unstaged)` })}
              actions={[
                { label: "Stage", icon: <Plus className="size-3.5" aria-hidden="true" />, onClick: () => onStage([e.path]) },
                { label: "Discard changes", icon: <Undo2 className="size-3.5" aria-hidden="true" />, onClick: () => onDiscard([e.path]), danger: true },
              ]}
            />
          ))}
        </Group>

        <Group title="New files" count={untracked.length}>
          {untracked.map((e) => (
            <FileRow
              key={`n-${e.path}`}
              entry={e}
              letter="?"
              active={isOpen(e.path, "untracked")}
              busy={busy}
              onOpen={() => onOpenFile({ target: { kind: "untracked" }, path: e.path, label: `${e.path} (new)` })}
              actions={[
                { label: "Stage", icon: <Plus className="size-3.5" aria-hidden="true" />, onClick: () => onStage([e.path]) },
                { label: "Delete the file", icon: <Undo2 className="size-3.5" aria-hidden="true" />, onClick: () => onDiscard([e.path]), danger: true },
              ]}
            />
          ))}
        </Group>
      </div>

      <div className="flex shrink-0 flex-col gap-2 border-t border-border/70 p-3">
        <textarea
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void submit();
          }}
          placeholder={merging ? "Merge commit message (leave empty for the default)" : amend ? "New message (leave empty to keep the old one)" : "Commit message"}
          aria-label="Commit message"
          className={cn(textareaClass, "min-h-[72px]")}
        />
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
            <input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} className="accent-[var(--primary)]" />
            Amend last commit
          </label>
          <PrimaryButton className="ml-auto" disabled={!canCommit} onClick={() => void submit()}>
            <FileText className="size-3.5" aria-hidden="true" />
            {amend ? "Amend" : "Commit"}
            {staged.length > 0 && !amend ? ` ${staged.length} file${staged.length === 1 ? "" : "s"}` : ""}
          </PrimaryButton>
        </div>
        {conflicted.length > 0 && <Pill tone="bad">Resolve {conflicted.length} conflict{conflicted.length === 1 ? "" : "s"} before committing</Pill>}
      </div>
    </div>
  );
};
