import React from "react";
import { ArrowDown, ArrowUp, FolderGit2, GitBranch, GitPullRequest, RefreshCw, Search } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { bridge } from "../../api/bridge";
import { describeBridgeError } from "../../types/api";
import type { PrBrief, RepoCard, RepoPrs } from "../../types/git";
import { Page, PageHeader } from "../../components/page/Page";
import { GhostButton, Pill, Seg, type Tone } from "../../components/page/kit";
import { fuzzyScore } from "../projects/ProjectPalette";
import { ago, attentionScore } from "./gitUi";

type Sort = "attention" | "recent" | "name";
type Group = "none" | "owner" | "project";

const PREFS_KEY = "tendril.git.view";

const loadPrefs = (): { sort: Sort; group: Group } => {
  try {
    const raw = JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as { sort?: Sort; group?: Group };
    return { sort: raw.sort ?? "attention", group: raw.group ?? "none" };
  } catch {
    return { sort: "attention", group: "none" };
  }
};

const CHECKS_TONE: Record<string, Tone> = { passing: "ok", failing: "bad", pending: "warn", none: "mute" };

const Chip: React.FC<{ tone?: Tone; title?: string; children: React.ReactNode }> = ({ tone = "mute", title, children }) => (
  <span title={title}>
    <Pill tone={tone}>{children}</Pill>
  </span>
);

const Stat: React.FC<{ label: string; value: React.ReactNode; warn?: boolean }> = ({ label, value, warn }) => (
  <div className="flex min-w-0 flex-col gap-0.5">
    <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">{label}</span>
    <span className={cn("truncate font-mono text-[13px]", warn ? "text-warning" : "text-foreground")}>{value}</span>
  </div>
);

const RepoCardView: React.FC<{ card: RepoCard; pr: PrBrief | undefined; onOpen: () => void }> = ({ card, pr, onOpen }) => {
  const s = card.summary;
  const dirty = s ? s.staged + s.unstaged + s.untracked : 0;
  const needs = attentionScore(s);

  return (
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        "group flex flex-col gap-3.5 rounded-[14px] border bg-card p-4 text-left shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] transition-colors hover:border-primary/40 hover:bg-secondary/40",
        needs >= 4 ? "border-warning/40" : "border-border",
      )}
    >
      <div className="flex items-center gap-2.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-success">
          <FolderGit2 className="size-4" aria-hidden="true" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-semibold text-foreground">{card.name}</div>
          <div className="truncate font-mono text-[11px] text-muted-foreground">
            {card.project}
            {card.owner ? ` · ${card.owner}` : ""}
          </div>
        </div>
        {!s ? (
          <Pill tone="bad">Unreadable</Pill>
        ) : s.conflicted > 0 ? (
          <Pill tone="bad">Conflicts</Pill>
        ) : s.state !== "clean" ? (
          <Pill tone="warn">{s.state === "merging" ? "Merging" : s.state === "rebasing" ? "Rebasing" : "In progress"}</Pill>
        ) : dirty > 0 ? (
          <Pill tone="warn">{dirty} uncommitted</Pill>
        ) : (
          <Pill tone="ok">Clean</Pill>
        )}
      </div>

      {s ? (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <span className="inline-flex min-w-0 items-center gap-1.5 rounded-md bg-muted px-2 py-1 font-mono text-[12px] text-foreground">
              <GitBranch className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="truncate">{s.detached ? `detached @ ${s.head?.slice(0, 7)}` : (s.branch ?? "no commits")}</span>
            </span>
            {s.ahead > 0 && (
              <Chip tone="info" title={`${s.ahead} commit(s) not pushed`}>
                <ArrowUp className="size-3" aria-hidden="true" /> {s.ahead}
              </Chip>
            )}
            {s.behind > 0 && (
              <Chip tone="warn" title={`${s.behind} commit(s) to pull`}>
                <ArrowDown className="size-3" aria-hidden="true" /> {s.behind}
              </Chip>
            )}
            {s.branch && !s.upstream && !s.detached && s.lastCommit && <Chip title="This branch is not on the remote yet">not pushed</Chip>}
            {pr && (
              <Chip tone={CHECKS_TONE[pr.checks]} title={`${pr.title} · checks ${pr.checks}`}>
                <GitPullRequest className="size-3" aria-hidden="true" /> #{pr.number}
                {pr.draft ? " draft" : ""}
              </Chip>
            )}
          </div>

          {dirty > 0 && (
            <div className="flex flex-wrap gap-1.5 font-mono text-[11px]">
              {s.staged > 0 && <span className="rounded bg-primary/12 px-1.5 py-0.5 text-success">{s.staged} staged</span>}
              {s.unstaged > 0 && <span className="rounded bg-warning/13 px-1.5 py-0.5 text-warning">{s.unstaged} modified</span>}
              {s.untracked > 0 && <span className="rounded bg-secondary px-1.5 py-0.5 text-muted-foreground">{s.untracked} new</span>}
              {s.conflicted > 0 && <span className="rounded bg-destructive/14 px-1.5 py-0.5 text-destructive">{s.conflicted} conflicted</span>}
            </div>
          )}

          <div className="min-h-[34px] text-[12.5px] leading-snug">
            {s.lastCommit ? (
              <>
                <div className="line-clamp-1 text-foreground">{s.lastCommit.subject}</div>
                <div className="font-mono text-[10.5px] text-muted-foreground">
                  {s.lastCommit.short} · {s.lastCommit.author} · {ago(s.lastCommit.at)}
                </div>
              </>
            ) : (
              <span className="text-muted-foreground">No commits yet.</span>
            )}
          </div>

          <div className="grid grid-cols-4 gap-3 border-t border-border/60 pt-3">
            <Stat label="Branches" value={s.localBranches} />
            <Stat label="Stale" value={s.staleBranches} warn={s.staleBranches > 3} />
            <Stat label="Stashes" value={s.stashes} />
            <Stat label="Worktrees" value={s.worktrees} />
          </div>
        </>
      ) : (
        <p className="m-0 text-[12.5px] text-destructive">{card.error ?? "This repository could not be read."}</p>
      )}
    </button>
  );
};

/** Every repository in your projects, the ones that need a look first. */
export const GitRepos: React.FC<{ projectFilter?: string; onOpenRepo: (id: string) => void }> = ({ projectFilter, onOpenRepo }) => {
  const [cards, setCards] = React.useState<RepoCard[] | null>(null);
  const [prs, setPrs] = React.useState<Record<string, RepoPrs>>({});
  const [error, setError] = React.useState<string | null>(null);
  const [query, setQuery] = React.useState("");
  const [{ sort, group }, setPrefs] = React.useState(loadPrefs);
  const [refreshing, setRefreshing] = React.useState(false);

  const load = React.useCallback(async () => {
    setRefreshing(true);
    try {
      setCards(await bridge.gitRepos());
      setError(null);
    } catch (e) {
      setError(describeBridgeError(e));
    } finally {
      setRefreshing(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [load]);

  // `gh` is slow, so the cards draw first and the pull requests arrive after.
  React.useEffect(() => {
    let live = true;
    const loadPrs = () =>
      bridge
        .gitPrs()
        .then((p) => live && setPrs(p))
        .catch(() => undefined);
    void loadPrs();
    const timer = window.setInterval(loadPrs, 60_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, []);

  const update = (next: Partial<{ sort: Sort; group: Group }>) =>
    setPrefs((prev) => {
      const merged = { ...prev, ...next };
      try {
        localStorage.setItem(PREFS_KEY, JSON.stringify(merged));
      } catch {
        /* the choice lasts for this session */
      }
      return merged;
    });

  const visible = React.useMemo(() => {
    const q = query.trim();
    const matched = (cards ?? []).filter(
      (c) =>
        (!projectFilter || c.project === projectFilter) &&
        (!q || fuzzyScore(q, `${c.name} ${c.project} ${c.owner ?? ""} ${c.summary?.branch ?? ""}`) !== null),
    );
    const by: Record<Sort, (a: RepoCard, b: RepoCard) => number> = {
      attention: (a, b) =>
        attentionScore(b.summary) - attentionScore(a.summary) || (b.summary?.lastCommit?.at ?? 0) - (a.summary?.lastCommit?.at ?? 0),
      recent: (a, b) => (b.summary?.lastCommit?.at ?? 0) - (a.summary?.lastCommit?.at ?? 0),
      name: (a, b) => a.name.localeCompare(b.name),
    };
    return [...matched].sort(by[sort]);
  }, [cards, query, sort, projectFilter]);

  const groups = React.useMemo(() => {
    if (group === "none") return [{ key: "", cards: visible }];
    const map = new Map<string, RepoCard[]>();
    for (const c of visible) {
      const key = group === "owner" ? (c.owner ?? "Local only") : c.project;
      map.set(key, [...(map.get(key) ?? []), c]);
    }
    return [...map.entries()]
      .sort(([a], [b]) => (a === "Local only" ? 1 : b === "Local only" ? -1 : a.localeCompare(b)))
      .map(([key, cs]) => ({ key, cards: cs }));
  }, [visible, group]);

  const prFor = (card: RepoCard): PrBrief | undefined => prs[card.id]?.prs.find((p) => p.branch === card.summary?.branch);
  const loose = (cards ?? []).filter((c) => (c.summary ? c.summary.staged + c.summary.unstaged + c.summary.untracked > 0 : false)).length;

  return (
    <Page testId="git-repos">
      <PageHeader
        title="Git"
        subtitle={
          cards
            ? `${cards.length} repositor${cards.length === 1 ? "y" : "ies"}${loose ? ` · ${loose} with uncommitted work` : " · nothing uncommitted"}${projectFilter ? ` · ${projectFilter}` : ""}`
            : "Reading your repositories…"
        }
        actions={
          <GhostButton onClick={() => void load()} disabled={refreshing}>
            <RefreshCw className={cn("size-3.5", refreshing && "animate-spin")} aria-hidden="true" />
            Refresh
          </GhostButton>
        }
      />

      <div className="flex flex-wrap items-center gap-3">
        <label className="flex h-9 min-w-[220px] flex-1 items-center gap-2 rounded-lg border border-border bg-card px-3 focus-within:border-primary/50 sm:max-w-[360px]">
          <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search repositories, projects or branches…"
            aria-label="Search repositories"
            className="min-w-0 flex-1 border-0 bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
          />
        </label>
        <Seg<Sort>
          label="Sort"
          value={sort}
          onChange={(v) => update({ sort: v })}
          options={[
            { value: "attention", label: "Needs attention" },
            { value: "recent", label: "Recent" },
            { value: "name", label: "Name" },
          ]}
        />
        <Seg<Group>
          label="Group"
          className="ml-auto"
          value={group}
          onChange={(v) => update({ group: v })}
          options={[
            { value: "none", label: "No grouping" },
            { value: "project", label: "By project" },
            { value: "owner", label: "By owner" },
          ]}
        />
      </div>

      {error && <div className="rounded-xl border border-destructive/40 bg-destructive/10 p-4 text-[13px] text-destructive">{error}</div>}
      {cards && visible.length === 0 && !error && (
        <div className="rounded-xl border border-dashed border-border px-6 py-12 text-center text-[13px] text-muted-foreground">
          {cards.length === 0 ? "No repositories yet. Add a project with a repository and it appears here." : `No repositories match “${query}”.`}
        </div>
      )}

      {groups.map((g) => (
        <section key={g.key || "all"} className="flex flex-col gap-3">
          {g.key && (
            <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
              {g.key}
              <span className="rounded-full bg-secondary px-1.5 text-[10.5px]">{g.cards.length}</span>
              <span className="h-px flex-1 bg-border/60" />
            </div>
          )}
          <div className="grid grid-cols-[repeat(auto-fill,minmax(330px,1fr))] gap-4">
            {g.cards.map((card) => (
              <RepoCardView key={card.id} card={card} pr={prFor(card)} onOpen={() => onOpenRepo(card.id)} />
            ))}
          </div>
        </section>
      ))}
    </Page>
  );
};
