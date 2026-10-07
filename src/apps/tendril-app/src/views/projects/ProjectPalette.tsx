import React from "react";
import { ArrowRight, CornerDownLeft, FolderGit2, GitBranch, History, LayoutGrid, Search, Settings, TrendingUp } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { Job, ProjectSummary } from "../../types/api";
import type { RepoCard } from "../../types/git";
import { bridge } from "../../api/bridge";
import { projectStatus } from "../../utils/projectStatus";
import { useRecentProjects } from "../../state/recentProjects";
import { setProjectIntent, type ProjectTab } from "../../state/projectIntent";
import { Dot, Kbd } from "../../components/page/kit";
import { useMissions } from "./ProjectsHomeView";
import { useManagerActivity } from "../../state/managerActivity";

interface Props {
  projects: ProjectSummary[];
  jobs: Job[];
  /** The project page currently open, so its panels can be offered. */
  currentProject: string | null;
  onClose: () => void;
  onNavigate: (nav: string, args?: unknown) => void;
}

interface Item {
  id: string;
  section: string;
  label: string;
  hint?: string;
  detail?: string;
  busy?: boolean;
  icon: React.ReactNode;
  run: () => void;
}

/** Subsequence match with a bonus for prefixes and word starts; null when it does not match. */
export const fuzzyScore = (query: string, text: string): number | null => {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 0;
  let score = 0;
  let ti = 0;
  for (let qi = 0; qi < q.length; qi++) {
    const found = t.indexOf(q[qi], ti);
    if (found === -1) return null;
    score += found === ti ? 3 : 1;
    if (found === 0 || /[\s\-_/.]/.test(t[found - 1] ?? "")) score += 4;
    ti = found + 1;
  }
  return score - t.length * 0.01;
};

const PANELS: { tab: ProjectTab; label: string }[] = [
  { tab: "overview", label: "Overview" },
  { tab: "missions", label: "Missions" },
  { tab: "tasks", label: "Tasks" },
  { tab: "models", label: "Models: agent and model per role" },
  { tab: "runtime", label: "Runtime: ports and Docker" },
  { tab: "memory", label: "Memory" },
];

/** ⌘K: find a project and jump into it, or go straight to one of its panels. Centred, keyboard-first. */
export const ProjectPalette: React.FC<Props> = ({ projects, jobs, currentProject, onClose, onNavigate }) => {
  const missions = useMissions(15_000);
  const recents = useRecentProjects();
  const activity = useManagerActivity();
  const [query, setQuery] = React.useState("");
  const [index, setIndex] = React.useState(0);
  const [owners, setOwners] = React.useState<Record<string, string | null>>({});
  const listRef = React.useRef<HTMLDivElement>(null);

  const [repos, setRepos] = React.useState<RepoCard[]>([]);

  React.useEffect(() => {
    bridge.getProjectOwners().then(setOwners).catch(() => undefined);
    bridge.gitRepos().then(setRepos).catch(() => undefined);
  }, []);

  const open = React.useCallback(
    (name: string, tab?: ProjectTab) => {
      if (tab) setProjectIntent({ tab });
      onNavigate(`project-${name}`);
      onClose();
    },
    [onNavigate, onClose],
  );

  const items = React.useMemo<Item[]>(() => {
    const byName = new Map(projects.map((p) => [p.name, p]));
    const mkProject = (p: ProjectSummary, section: string, icon: React.ReactNode): Item => {
      const s = projectStatus(p.name, missions, jobs, activity[p.name]?.working ?? false);
      return {
        id: `project:${section}:${p.name}`,
        section,
        label: p.name,
        hint: owners[p.name] ?? undefined,
        detail: s.currentTask ?? s.headline,
        busy: s.busy,
        icon,
        run: () => open(p.name),
      };
    };

    const result: Item[] = [];
    const q = query.trim();

    if (!q) {
      for (const r of recents) {
        const p = byName.get(r.name);
        if (p) result.push(mkProject(p, "Jump back in", <History className="size-4" aria-hidden="true" />));
      }
    }

    const scored = projects
      .map((p) => ({ p, score: fuzzyScore(q, `${p.name} ${owners[p.name] ?? ""}`) }))
      .filter((x): x is { p: ProjectSummary; score: number } => x.score !== null)
      .sort((a, b) => b.score - a.score || a.p.name.localeCompare(b.p.name));
    for (const { p } of scored) result.push(mkProject(p, "Projects", <FolderGit2 className="size-4" aria-hidden="true" />));

    if (currentProject) {
      for (const panel of PANELS) {
        if (fuzzyScore(q, `${panel.label} ${currentProject} panel`) === null) continue;
        result.push({
          id: `panel:${panel.tab}`,
          section: `In ${currentProject}`,
          label: panel.label,
          icon: <ArrowRight className="size-4" aria-hidden="true" />,
          run: () => {
            setProjectIntent({ tab: panel.tab });
            onClose();
          },
        });
      }
    }

    const pages: { id: string; label: string; icon: React.ReactNode }[] = [
      { id: "projects", label: "All projects", icon: <FolderGit2 className="size-4" aria-hidden="true" /> },
      { id: "git", label: "Git: all repositories", icon: <GitBranch className="size-4" aria-hidden="true" /> },
      { id: "dashboard", label: "Dashboard", icon: <LayoutGrid className="size-4" aria-hidden="true" /> },
      { id: "insights", label: "Insights", icon: <TrendingUp className="size-4" aria-hidden="true" /> },
      { id: "settings", label: "Settings", icon: <Settings className="size-4" aria-hidden="true" /> },
    ];
    // Repositories, once something is typed: jump straight into a repo's workspace.
    if (q) {
      const matches = repos
        .map((repo) => ({ repo, score: fuzzyScore(q, `${repo.name} ${repo.project} ${repo.summary?.branch ?? ""} git`) }))
        .filter((m): m is { repo: RepoCard; score: number } => m.score !== null)
        .sort((a, b) => b.score - a.score)
        .slice(0, 8);
      for (const { repo } of matches) {
        const s = repo.summary;
        const dirty = s ? s.staged + s.unstaged + s.untracked : 0;
        result.push({
          id: `repo:${repo.id}`,
          section: "Git repositories",
          label: repo.name,
          hint: repo.project,
          detail: s ? `${s.branch ?? "detached"}${dirty ? ` · ${dirty} uncommitted` : ""}${s.ahead ? ` · ↑${s.ahead}` : ""}${s.behind ? ` · ↓${s.behind}` : ""}` : "unreadable",
          icon: <GitBranch className="size-4" aria-hidden="true" />,
          run: () => {
            onNavigate("git", { repo: repo.id });
            onClose();
          },
        });
      }
    }

    for (const page of pages) {
      if (fuzzyScore(q, page.label) === null) continue;
      result.push({
        id: `page:${page.id}`,
        section: "Go to",
        label: page.label,
        icon: page.icon,
        run: () => {
          onNavigate(page.id);
          onClose();
        },
      });
    }
    return result;
  }, [projects, missions, jobs, activity, owners, repos, query, recents, currentProject, open, onNavigate, onClose]);

  React.useEffect(() => setIndex(0), [query]);
  React.useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
      e.preventDefault();
      setIndex((i) => Math.min(items.length - 1, i + 1));
    } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
      e.preventDefault();
      setIndex((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      items[index]?.run();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  let lastSection = "";
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/55 px-4 pt-[14vh] backdrop-blur-[3px]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      role="presentation"
    >
      <div
        role="dialog"
        aria-label="Jump to project"
        aria-modal="true"
        onKeyDown={onKeyDown}
        className="flex max-h-[60vh] w-full max-w-[600px] flex-col overflow-hidden rounded-2xl border border-[#22303d] bg-card shadow-[0_0_0_4px_rgba(0,204,146,0.04),0_30px_80px_-20px_rgba(0,0,0,0.8)]"
      >
        <div className="flex items-center gap-3 border-b border-border/70 px-4">
          <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to a project…"
            aria-label="Search projects"
            className="h-12 min-w-0 flex-1 border-0 bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          <Kbd>esc</Kbd>
        </div>

        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-2" role="listbox">
          {items.length === 0 && (
            <div className="px-3 py-8 text-center text-[13px] text-muted-foreground">No projects match “{query}”.</div>
          )}
          {items.map((item, i) => {
            const header = item.section !== lastSection ? item.section : null;
            lastSection = item.section;
            return (
              <React.Fragment key={item.id}>
                {header && (
                  <div className="px-3 pb-1 pt-3 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground first:pt-1">
                    {header}
                  </div>
                )}
                <button
                  type="button"
                  role="option"
                  aria-selected={i === index}
                  data-index={i}
                  onMouseMove={() => setIndex(i)}
                  onClick={item.run}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left",
                    i === index ? "bg-secondary" : "hover:bg-secondary/50",
                  )}
                >
                  <span className={cn("shrink-0", i === index ? "text-success" : "text-muted-foreground")}>{item.icon}</span>
                  <span className="shrink-0 text-[13.5px] text-foreground">{item.label}</span>
                  {item.hint && <span className="shrink-0 font-mono text-[11px] text-muted-foreground">{item.hint}</span>}
                  {item.detail && (
                    <span className="flex min-w-0 flex-1 items-center justify-end gap-1.5 text-[12px] text-muted-foreground">
                      {item.busy && <span className="text-success"><Dot live /></span>}
                      <span className="truncate">{item.detail}</span>
                    </span>
                  )}
                  {i === index && <CornerDownLeft className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />}
                </button>
              </React.Fragment>
            );
          })}
        </div>

        <div className="flex gap-4 border-t border-border/70 px-4 py-2 font-mono text-[10.5px] text-muted-foreground">
          <span><Kbd>↑</Kbd> <Kbd>↓</Kbd> move</span>
          <span><Kbd>⏎</Kbd> open</span>
          <span className="ml-auto"><Kbd>⌘K</Kbd> toggle</span>
        </div>
      </div>
    </div>
  );
};
