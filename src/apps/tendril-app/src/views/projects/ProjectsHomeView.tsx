import React from "react";
import { FolderGit2, History, Plus, Search } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { Job, Mission, ProjectSummary } from "../../types/api";
import { bridge } from "../../api/bridge";
import { projectStatus, type ProjectStatus } from "../../utils/projectStatus";
import { formatAge } from "../../utils/commandCenter";
import { useRecentProjects } from "../../state/recentProjects";
import { useManagerActivity } from "../../state/managerActivity";
import { Page, PageHeader } from "../../components/page/Page";
import { Dot, Kbd, Pill, PrimaryButton, Seg } from "../../components/page/kit";
import { fuzzyScore } from "./ProjectPalette";

interface Props {
  projects: ProjectSummary[];
  jobs: Job[];
  onOpenProject: (name: string) => void;
  onAddProject?: () => void;
}

export const useMissions = (intervalMs = 15_000): Mission[] => {
  const [missions, setMissions] = React.useState<Mission[]>([]);
  React.useEffect(() => {
    let live = true;
    const load = () =>
      bridge
        .listMissions()
        .then((m) => live && setMissions(m))
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, intervalMs);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [intervalMs]);
  return missions;
};

type Sort = "active" | "recent" | "name" | "spend";
type Group = "none" | "owner";

const PREFS_KEY = "tendril.projects.view";

const loadPrefs = (): { sort: Sort; group: Group } => {
  try {
    const raw = JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as { sort?: Sort; group?: Group };
    return { sort: raw.sort ?? "active", group: raw.group ?? "none" };
  } catch {
    return { sort: "active", group: "none" };
  }
};

const SEGMENT: Record<string, string> = {
  Passed: "bg-primary",
  Skipped: "bg-primary/40",
  Executing: "bg-info animate-pulse",
  Judging: "bg-violet animate-pulse",
  Pending: "bg-muted",
};

const ago = (iso: string | null) => (iso ? `${formatAge(Math.max(0, Date.now() - new Date(iso).getTime()))} ago` : "no activity");

/** One segment per milestone across the project's live missions, so progress also shows what is moving. */
const ProgressStrip: React.FC<{ status: ProjectStatus }> = ({ status }) => {
  const milestones = status.live.flatMap((m) => m.milestones);
  if (milestones.length === 0) {
    return <div className="h-1.5 rounded-full bg-muted" aria-hidden="true" />;
  }
  return (
    <div className="flex gap-[3px]" aria-hidden="true">
      {milestones.slice(0, 40).map((ms, i) => (
        <div key={`${ms.id}-${i}`} title={`${ms.title} · ${ms.state}`} className={cn("h-1.5 flex-1 rounded-full", SEGMENT[ms.state] ?? "bg-muted")} />
      ))}
    </div>
  );
};

const Stat: React.FC<{ label: string; value: React.ReactNode }> = ({ label, value }) => (
  <div className="flex min-w-0 flex-col gap-0.5">
    <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">{label}</span>
    <span className="truncate font-mono text-[13px] text-foreground">{value}</span>
  </div>
);

const ProjectCard: React.FC<{
  project: ProjectSummary;
  status: ProjectStatus;
  owner?: string | null;
  onOpen: () => void;
}> = ({ project, status, owner, onOpen }) => {
  const needsYou = status.attention.length > 0;
  const live = status.live.length > 0;
  const livePct = (() => {
    const ms = status.live.flatMap((m) => m.milestones);
    return ms.length ? Math.round((ms.filter((m) => m.state === "Passed" || m.state === "Skipped").length / ms.length) * 100) : 0;
  })();

  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex flex-col gap-3.5 rounded-[14px] border border-border bg-card p-4 text-left shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] transition-colors hover:border-primary/40 hover:bg-secondary/40"
    >
      <div className="flex items-center gap-2.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-success">
          <FolderGit2 className="size-4" aria-hidden="true" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-semibold text-foreground">{project.name}</div>
          <div className="truncate font-mono text-[11px] text-muted-foreground">
            {owner ? `${owner} · ` : ""}
            {project.repos[0]?.split("/").slice(-1)[0] ?? "no repo"}
          </div>
        </div>
        {needsYou ? (
          <Pill tone="warn">Needs you</Pill>
        ) : status.busy ? (
          <Pill tone="ok" dot live>Working</Pill>
        ) : (
          <Pill tone="mute">Idle</Pill>
        )}
      </div>

      <div className="flex min-h-[34px] items-start gap-2 text-[12.5px] leading-snug">
        {status.busy && <span className="mt-1 text-success"><Dot live /></span>}
        <span className={cn("line-clamp-2", status.currentTask ? "text-foreground" : "text-muted-foreground")}>
          {status.currentTask ?? status.headline}
        </span>
      </div>

      <div className="flex flex-col gap-1.5">
        <ProgressStrip status={status} />
        <div className="flex justify-between font-mono text-[10.5px] text-muted-foreground">
          <span>{live ? `${livePct}% in progress` : status.milestonesTotal ? `${status.milestonesPassed}/${status.milestonesTotal} milestones done` : "no goals yet"}</span>
          <span>{status.milestonesPassed}/{status.milestonesTotal}</span>
        </div>
      </div>

      <div className="grid grid-cols-4 gap-3 border-t border-border/60 pt-3">
        <Stat label="Missions" value={status.live.length} />
        <Stat label="Tasks" value={status.taskCount} />
        <Stat label="Spend" value={`$${status.cost.toFixed(2)}`} />
        <Stat label="Active" value={ago(status.updated)} />
      </div>
    </button>
  );
};

export const ProjectsHomeView: React.FC<Props> = ({ projects, jobs, onOpenProject, onAddProject }) => {
  const missions = useMissions();
  const recents = useRecentProjects();
  const activity = useManagerActivity();
  const [query, setQuery] = React.useState("");
  const [{ sort, group }, setPrefs] = React.useState(loadPrefs);
  const [owners, setOwners] = React.useState<Record<string, string | null>>({});

  React.useEffect(() => {
    bridge.getProjectOwners().then(setOwners).catch(() => undefined);
  }, []);

  const update = (next: Partial<{ sort: Sort; group: Group }>) =>
    setPrefs((prev) => {
      const merged = { ...prev, ...next };
      try {
        localStorage.setItem(PREFS_KEY, JSON.stringify(merged));
      } catch {
        /* storage unavailable: the choice lasts for this session */
      }
      return merged;
    });

  const rows = React.useMemo(
    () =>
      projects.map((project) => ({
        project,
        status: projectStatus(project.name, missions, jobs, activity[project.name]?.working ?? false),
        owner: owners[project.name] ?? null,
      })),
    [projects, missions, jobs, owners, activity],
  );

  const visible = React.useMemo(() => {
    const q = query.trim();
    const matched = rows.filter((r) => !q || fuzzyScore(q, `${r.project.name} ${r.owner ?? ""}`) !== null);
    const attention = (r: (typeof rows)[number]) => (r.status.attention.length ? 1 : 0);
    const by: Record<Sort, (a: (typeof rows)[number], b: (typeof rows)[number]) => number> = {
      active: (a, b) =>
        Number(b.status.busy) - Number(a.status.busy) ||
        attention(b) - attention(a) ||
        (b.status.updated ?? "").localeCompare(a.status.updated ?? "") ||
        a.project.name.localeCompare(b.project.name),
      recent: (a, b) => (b.status.updated ?? "").localeCompare(a.status.updated ?? "") || a.project.name.localeCompare(b.project.name),
      name: (a, b) => a.project.name.localeCompare(b.project.name),
      spend: (a, b) => b.status.cost - a.status.cost || a.project.name.localeCompare(b.project.name),
    };
    return [...matched].sort(by[sort]);
  }, [rows, query, sort]);

  const groups = React.useMemo(() => {
    if (group === "none") return [{ key: "", rows: visible }];
    const map = new Map<string, typeof visible>();
    for (const r of visible) {
      const key = r.owner ?? "Local only";
      map.set(key, [...(map.get(key) ?? []), r]);
    }
    return [...map.entries()]
      .sort(([a], [b]) => (a === "Local only" ? 1 : b === "Local only" ? -1 : a.localeCompare(b)))
      .map(([key, rs]) => ({ key, rows: rs }));
  }, [visible, group]);

  const jumpBack = React.useMemo(() => {
    const byName = new Map(rows.map((r) => [r.project.name, r]));
    return recents.map((r) => byName.get(r.name)).filter((r): r is (typeof rows)[number] => Boolean(r)).slice(0, 3);
  }, [recents, rows]);

  return (
    <Page testId="projects-home">
      <PageHeader
        title="Projects"
        subtitle="Each project has its own manager. Open one to see what it is doing."
        actions={
          onAddProject && (
            <PrimaryButton onClick={onAddProject}>
              <Plus className="size-3.5" aria-hidden="true" />
              Add project
            </PrimaryButton>
          )
        }
      />

      {jumpBack.length > 0 && !query && (
        <section className="flex flex-col gap-2.5" aria-label="Jump back in">
          <div className="flex items-center gap-2 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
            <History className="size-3.5" aria-hidden="true" />
            Jump back in
          </div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-3">
            {jumpBack.map(({ project, status }, i) => (
              <button
                key={project.name}
                type="button"
                onClick={() => onOpenProject(project.name)}
                className={cn(
                  "flex items-center gap-3 rounded-xl border bg-card px-4 py-3 text-left transition-colors hover:bg-secondary/50",
                  i === 0 ? "border-primary/40 shadow-[0_0_0_3px_rgba(0,204,146,0.06)]" : "border-border",
                )}
              >
                <span className={cn("shrink-0", status.busy ? "text-success" : "text-muted-foreground")}>
                  <Dot live={status.busy} />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13.5px] font-medium text-foreground">{project.name}</div>
                  <div className="truncate text-[12px] text-muted-foreground">{status.currentTask ?? status.headline}</div>
                </div>
                <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{status.live.length ? `${status.progress}%` : "resume"}</span>
              </button>
            ))}
          </div>
        </section>
      )}

      <div className="flex flex-wrap items-center gap-3">
        <label className="relative flex h-9 min-w-[220px] flex-1 items-center gap-2 rounded-lg border border-border bg-card px-3 focus-within:border-primary/50 sm:max-w-[360px]">
          <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search projects or owners…"
            aria-label="Search projects"
            className="min-w-0 flex-1 border-0 bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          {!query && <Kbd>⌘K</Kbd>}
        </label>
        <Seg<Sort>
          label="Sort"
          value={sort}
          onChange={(v) => update({ sort: v })}
          options={[
            { value: "active", label: "Active first" },
            { value: "recent", label: "Recent" },
            { value: "spend", label: "Spend" },
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
            { value: "owner", label: "By owner" },
          ]}
        />
      </div>

      {visible.length === 0 && (
        <div className="rounded-xl border border-dashed border-border px-6 py-12 text-center text-[13px] text-muted-foreground">
          No projects match “{query}”.
        </div>
      )}

      {groups.map((g) => (
        <section key={g.key || "all"} className="flex flex-col gap-3">
          {g.key && (
            <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
              {g.key}
              <span className="rounded-full bg-secondary px-1.5 text-[10.5px]">{g.rows.length}</span>
              <span className="h-px flex-1 bg-border/60" />
            </div>
          )}
          <div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-4">
            {g.rows.map(({ project, status, owner }) => (
              <ProjectCard key={project.name} project={project} status={status} owner={owner} onOpen={() => onOpenProject(project.name)} />
            ))}
          </div>
        </section>
      ))}
    </Page>
  );
};
