import React from "react";
import { ChevronDown, ChevronRight, GitBranch, GitPullRequest, Layers, Search, Tag } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { PrBrief, RefInfo, RepoDetail, RepoPrs } from "../../types/git";
import { openUrl } from "../../utils/opener";
import { useContextMenu, type MenuItem } from "./gitUi";

export type BranchAction =
  | { type: "checkout"; name: string }
  | { type: "checkoutRemote"; name: string }
  | { type: "merge"; name: string }
  | { type: "rebase"; name: string }
  | { type: "push"; name: string }
  | { type: "delete"; name: string; merged: boolean }
  | { type: "rename"; name: string }
  | { type: "deleteRemote"; remote: string; name: string }
  | { type: "pr"; name: string }
  | { type: "branchFrom"; hash: string }
  | { type: "tagAt"; hash: string }
  | { type: "deleteTag"; name: string }
  | { type: "checkoutCommit"; hash: string }
  | { type: "stash"; op: "pop" | "apply" | "drop"; index: number }
  | { type: "worktreeRemove"; path: string };

const Section: React.FC<{ title: string; count: number; icon: React.ReactNode; defaultOpen?: boolean; children: React.ReactNode }> = ({
  title,
  count,
  icon,
  defaultOpen = true,
  children,
}) => {
  const [open, setOpen] = React.useState(defaultOpen);
  return (
    <section className="flex flex-col">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1.5 px-3 py-1.5 text-left font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground hover:text-foreground"
        aria-expanded={open}
      >
        {open ? <ChevronDown className="size-3" aria-hidden="true" /> : <ChevronRight className="size-3" aria-hidden="true" />}
        {icon}
        {title}
        <span className="ml-auto rounded-full bg-secondary px-1.5 text-[10px]">{count}</span>
      </button>
      {open && <div className="flex flex-col pb-1">{children}</div>}
    </section>
  );
};

const Row: React.FC<{
  active?: boolean;
  dimmed?: boolean;
  onClick?: () => void;
  onDoubleClick?: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  title?: string;
  children: React.ReactNode;
}> = ({ active, dimmed, onClick, onDoubleClick, onContextMenu, title, children }) => (
  <div
    role="button"
    tabIndex={0}
    title={title}
    onClick={onClick}
    onDoubleClick={onDoubleClick}
    onContextMenu={onContextMenu}
    onKeyDown={(e) => e.key === "Enter" && onClick?.()}
    className={cn(
      "mx-1.5 flex cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-[12.5px] hover:bg-secondary/60",
      active && "bg-secondary",
      dimmed && "opacity-60",
    )}
  >
    {children}
  </div>
);

const Counts: React.FC<{ r: RefInfo }> = ({ r }) => (
  <span className="flex shrink-0 gap-1 font-mono text-[10.5px]">
    {r.ahead > 0 && <span className="text-info">↑{r.ahead}</span>}
    {r.behind > 0 && <span className="text-warning">↓{r.behind}</span>}
    {r.gone && <span className="text-destructive" title="Its remote branch was deleted">gone</span>}
  </span>
);

const CHECK_DOT: Record<string, string> = { passing: "bg-success", failing: "bg-destructive", pending: "bg-warning", none: "bg-muted-foreground/50" };

/** Branches, remotes, tags, stashes, worktrees and pull requests, each with its right-click menu. */
export const BranchSidebar: React.FC<{
  detail: RepoDetail;
  prs: RepoPrs | null;
  filter: string | null;
  selectedHash: string | null;
  busy: boolean;
  onSelectRef: (hash: string) => void;
  onFilter: (branch: string | null) => void;
  onAction: (action: BranchAction) => void;
  onOpenWorktree: (id: string) => void;
}> = ({ detail, prs, filter, selectedHash, busy, onSelectRef, onFilter, onAction, onOpenWorktree }) => {
  const [query, setQuery] = React.useState("");
  const menu = useContextMenu();
  const { refs, stashes, worktrees } = detail.overview;
  const current = detail.overview.status.branch;
  const q = query.trim().toLowerCase();
  const match = (name: string) => !q || name.toLowerCase().includes(q);

  const local = refs.filter((r) => r.kind === "local" && match(r.name));
  const remote = refs.filter((r) => r.kind === "remote" && match(r.name));
  const tags = refs.filter((r) => r.kind === "tag" && match(r.name));
  const prByBranch = new Map<string, PrBrief>((prs?.prs ?? []).map((p) => [p.branch, p]));

  const localMenu = (r: RefInfo): MenuItem[] => [
    { label: "Check out", onClick: () => onAction({ type: "checkout", name: r.name }), disabled: r.current || busy },
    { label: current ? `Merge into ${current}` : "Merge into current", onClick: () => onAction({ type: "merge", name: r.name }), disabled: r.current || busy },
    { label: current ? `Rebase ${current} onto this` : "Rebase onto this", onClick: () => onAction({ type: "rebase", name: r.name }), disabled: r.current || busy },
    { label: "Push", onClick: () => onAction({ type: "push", name: r.name }), disabled: busy, separator: true },
    { label: "Create pull request…", onClick: () => onAction({ type: "pr", name: r.name }), disabled: busy || !!prByBranch.get(r.name) },
    { label: filter === r.name ? "Show all branches" : "Show only this branch", onClick: () => onFilter(filter === r.name ? null : r.name), separator: true },
    { label: "Rename…", onClick: () => onAction({ type: "rename", name: r.name }), disabled: busy },
    { label: "Delete…", onClick: () => onAction({ type: "delete", name: r.name, merged: r.merged }), danger: true, disabled: r.current || busy },
  ];

  return (
    <aside className="flex min-h-0 w-full flex-col border-r border-border/70" aria-label="Branches">
      <label className="m-2 flex h-8 items-center gap-2 rounded-lg border border-border bg-card px-2.5 focus-within:border-primary/50">
        <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter branches, tags…"
          aria-label="Filter branches"
          className="min-w-0 flex-1 border-0 bg-transparent text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground"
        />
      </label>
      {filter && (
        <button type="button" onClick={() => onFilter(null)} className="mx-2 mb-1 rounded-md bg-info/12 px-2 py-1 text-left text-[11.5px] text-info hover:bg-info/18">
          Showing only {filter} · click to show all
        </button>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto pb-3">
        <Section title="Local" count={local.length} icon={<GitBranch className="size-3" aria-hidden="true" />}>
          {local.map((r) => {
            const link = detail.links[r.name];
            const pr = prByBranch.get(r.name);
            return (
              <Row
                key={r.fullRef}
                active={selectedHash === r.hash || filter === r.name}
                onClick={() => onSelectRef(r.hash)}
                onDoubleClick={() => !r.current && onAction({ type: "checkout", name: r.name })}
                onContextMenu={(e) => menu.open(e, localMenu(r))}
                title={r.subject}
              >
                <span className={cn("size-1.5 shrink-0 rounded-full", r.current ? "bg-success shadow-[0_0_0_3px_rgba(0,204,146,0.2)]" : "bg-transparent")} aria-hidden="true" />
                <span className={cn("min-w-0 flex-1 truncate font-mono", r.current ? "font-semibold text-foreground" : "text-foreground/90")}>{r.name}</span>
                {link && (
                  <span className="shrink-0 rounded bg-violet/14 px-1 font-mono text-[9.5px] uppercase text-violet" title={`${link.kind === "mission" ? "Mission" : "Plan"}: ${link.title}`}>
                    {link.kind === "mission" ? "mission" : "plan"}
                  </span>
                )}
                {pr && <span className={cn("size-1.5 shrink-0 rounded-full", CHECK_DOT[pr.checks])} title={`PR #${pr.number}: checks ${pr.checks}`} />}
                <Counts r={r} />
                {r.merged && !r.current && <span className="shrink-0 font-mono text-[9.5px] text-muted-foreground" title="Already merged: safe to delete">merged</span>}
              </Row>
            );
          })}
          {local.length === 0 && <div className="px-4 py-1 text-[11.5px] text-muted-foreground">None.</div>}
        </Section>

        <Section title="Remote" count={remote.length} icon={<GitBranch className="size-3" aria-hidden="true" />} defaultOpen={remote.length <= 30}>
          {remote.map((r) => {
            const [remoteName, ...rest] = r.name.split("/");
            const branch = rest.join("/");
            const hasLocal = refs.some((x) => x.kind === "local" && x.name === branch);
            return (
              <Row
                key={r.fullRef}
                active={selectedHash === r.hash}
                onClick={() => onSelectRef(r.hash)}
                onDoubleClick={() => !hasLocal && onAction({ type: "checkoutRemote", name: r.name })}
                onContextMenu={(e) =>
                  menu.open(e, [
                    { label: hasLocal ? "A local branch exists" : "Check out as a local branch", onClick: () => onAction({ type: "checkoutRemote", name: r.name }), disabled: hasLocal || busy },
                    { label: current ? `Merge into ${current}` : "Merge into current", onClick: () => onAction({ type: "merge", name: r.name }), disabled: busy },
                    { label: "Delete on the remote…", onClick: () => onAction({ type: "deleteRemote", remote: remoteName, name: branch }), danger: true, separator: true, disabled: busy },
                  ])
                }
                title={r.subject}
              >
                <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{remoteName}/</span>
                <span className="min-w-0 flex-1 truncate font-mono text-foreground/90">{branch}</span>
              </Row>
            );
          })}
        </Section>

        <Section title="Tags" count={tags.length} icon={<Tag className="size-3" aria-hidden="true" />} defaultOpen={false}>
          {tags.map((r) => (
            <Row
              key={r.fullRef}
              active={selectedHash === r.hash}
              onClick={() => onSelectRef(r.hash)}
              onContextMenu={(e) =>
                menu.open(e, [
                  { label: "Check out (detached)", onClick: () => onAction({ type: "checkoutCommit", hash: r.hash }), disabled: busy },
                  { label: "Delete tag…", onClick: () => onAction({ type: "deleteTag", name: r.name }), danger: true, separator: true, disabled: busy },
                ])
              }
            >
              <span className="min-w-0 flex-1 truncate font-mono text-warning">{r.name}</span>
            </Row>
          ))}
        </Section>

        <Section title="Stashes" count={stashes.length} icon={<Layers className="size-3" aria-hidden="true" />} defaultOpen={stashes.length > 0}>
          {stashes.map((s) => (
            <Row
              key={s.index}
              onContextMenu={(e) =>
                menu.open(e, [
                  { label: "Apply and keep", onClick: () => onAction({ type: "stash", op: "apply", index: s.index }), disabled: busy },
                  { label: "Pop (apply and remove)", onClick: () => onAction({ type: "stash", op: "pop", index: s.index }), disabled: busy },
                  { label: "Drop…", onClick: () => onAction({ type: "stash", op: "drop", index: s.index }), danger: true, separator: true, disabled: busy },
                ])
              }
              title="Right-click to apply, pop or drop"
            >
              <span className="min-w-0 flex-1 truncate text-foreground/90">{s.message}</span>
            </Row>
          ))}
          {stashes.length === 0 && <div className="px-4 py-1 text-[11.5px] text-muted-foreground">Nothing stashed.</div>}
        </Section>

        <Section title="Worktrees" count={worktrees.length} icon={<Layers className="size-3" aria-hidden="true" />} defaultOpen={worktrees.length > 1}>
          {worktrees.map((w) => {
            const id = detail.worktreeIds[w.path];
            const label = w.path.split("/").slice(-2).join("/");
            return (
              <Row
                key={w.path}
                title={w.path}
                onClick={() => !w.main && id && onOpenWorktree(id)}
                onContextMenu={(e) =>
                  w.main
                    ? undefined
                    : menu.open(e, [
                        { label: "Open as a repository", onClick: () => id && onOpenWorktree(id), disabled: !id },
                        { label: "Remove worktree…", onClick: () => onAction({ type: "worktreeRemove", path: w.path }), danger: true, separator: true, disabled: busy },
                      ])
                }
              >
                <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-foreground/90">{w.main ? "this checkout" : label}</span>
                <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{w.branch ?? (w.detached ? "detached" : "")}</span>
                {w.locked && <span className="shrink-0 text-[10px] text-warning">locked</span>}
              </Row>
            );
          })}
        </Section>

        <Section title="Pull requests" count={prs?.prs.length ?? 0} icon={<GitPullRequest className="size-3" aria-hidden="true" />} defaultOpen={(prs?.prs.length ?? 0) > 0}>
          {prs === null && <div className="px-4 py-1 text-[11.5px] text-muted-foreground">Loading…</div>}
          {prs?.error && prs.prs.length === 0 && <div className="px-4 py-1 text-[11.5px] leading-snug text-muted-foreground">{prs.error}</div>}
          {prs?.prs.map((p) => (
            <Row key={p.number} title={p.title} onClick={() => void openUrl(p.url)}>
              <span className={cn("size-1.5 shrink-0 rounded-full", CHECK_DOT[p.checks])} aria-hidden="true" />
              <span className="shrink-0 font-mono text-[11px] text-muted-foreground">#{p.number}</span>
              <span className="min-w-0 flex-1 truncate text-foreground/90">{p.title}</span>
              {p.draft && <span className="shrink-0 font-mono text-[9.5px] text-muted-foreground">draft</span>}
            </Row>
          ))}
        </Section>
      </div>
      {menu.element}
    </aside>
  );
};
