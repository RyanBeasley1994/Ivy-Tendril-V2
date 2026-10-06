import React from "react";
import { ArrowUp, RefreshCw } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { useTranslation, type TFunction } from "../i18n";
import type { Job, Mission, PlanSummary, ProjectSummary } from "../types/api";
import { bridge } from "../api/bridge";
import { useDashboardAnalytics } from "../hooks/useDashboardAnalytics";
import { projectStatus, type ProjectStatus } from "../utils/projectStatus";
import { formatAge } from "../utils/commandCenter";
import { useRecentProjects } from "../state/recentProjects";
import { useManagerActivity } from "../state/managerActivity";
import { Card, Dot, GhostButton, Pill, PrimaryButton, PrimaryKbd } from "../components/page/kit";
import { HeatmapCard } from "./command/panels";

interface DashboardViewProps {
  plans: PlanSummary[];
  jobs: Job[];
  projects?: ProjectSummary[];
  /** Opens a project's page, optionally on one of its missions or with text drafted for its manager. */
  onOpenProject?: (name: string, options?: { missionId?: string; draft?: string }) => void;
  /** Re-reads plans and jobs from the daemon. */
  onRefresh?: () => void;
  /** False while the service is unreachable. */
  serviceOnline?: boolean;
  // The props below are what the old plan-centred dashboard took; the App still passes them.
  onSelectJob?: (jobId: string) => void;
  onSelectPlan?: (planId: string) => void;
  onSelectMission?: (missionId: string) => void;
  onNavigate?: (navId: string) => void;
  onNewPlan?: (description?: string, project?: string) => void;
  onStopAll?: () => void;
}

const buildGreeting = (now: Date, t: TFunction<"dashboard">): string => {
  const hour = now.getHours();
  if (hour >= 5 && hour < 12) return t("command.greeting.morning");
  if (hour >= 12 && hour < 17) return t("command.greeting.afternoon");
  return t("command.greeting.evening");
};

const SEGMENT: Record<string, string> = {
  Passed: "bg-primary",
  Skipped: "bg-primary/40",
  Executing: "bg-info animate-pulse",
  Judging: "bg-violet animate-pulse",
  Pending: "bg-muted",
};

const ago = (iso: string | null | undefined) =>
  iso ? `${formatAge(Math.max(0, Date.now() - new Date(iso).getTime()))} ago` : "no activity";

const Strip: React.FC<{ status: ProjectStatus }> = ({ status }) => {
  const milestones = status.live.flatMap((m) => m.milestones).slice(0, 30);
  if (milestones.length === 0) return <div className="h-1.5 rounded-full bg-muted" aria-hidden="true" />;
  return (
    <div className="flex gap-[3px]" aria-hidden="true">
      {milestones.map((ms, i) => (
        <div key={`${ms.id}-${i}`} className={cn("h-1.5 flex-1 rounded-full", SEGMENT[ms.state] ?? "bg-muted")} />
      ))}
    </div>
  );
};

/** Tell a project's manager something: opens its chat with the text drafted, ready to send. */
const AskManager: React.FC<{
  projects: ProjectSummary[];
  onSubmit: (project: string, text: string) => void;
}> = ({ projects, onSubmit }) => {
  const recents = useRecentProjects();
  const [text, setText] = React.useState("");
  const [project, setProject] = React.useState("");
  const chosen = project || recents.find((r) => projects.some((p) => p.name === r.name))?.name || projects[0]?.name || "";

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!chosen) return;
    onSubmit(chosen, text.trim());
    setText("");
  };

  return (
    <form
      onSubmit={submit}
      className="flex shrink-0 flex-col gap-3 rounded-xl border border-[#22303d] bg-card py-3.5 pl-4 pr-3.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_0_0_4px_rgba(0,204,146,0.035),0_18px_40px_-24px_rgba(0,204,146,0.35)]"
    >
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="Tell a manager what you want done…"
        aria-label="Message a project manager"
        className="min-w-0 border-0 bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground"
      />
      <div className="flex flex-wrap items-center gap-2">
        <label className="inline-flex h-[26px] items-center gap-1.5 rounded-[7px] border border-border bg-muted pl-2 pr-1 text-xs text-foreground">
          <span className="text-muted-foreground">to</span>
          <select
            value={chosen}
            onChange={(e) => setProject(e.target.value)}
            aria-label="Project"
            className="bg-transparent pr-1 text-xs outline-none"
          >
            {projects.map((p) => (
              <option key={p.name} value={p.name}>{p.name}</option>
            ))}
          </select>
        </label>
        <PrimaryButton type="submit" className="ml-auto h-8" disabled={!chosen}>
          Open chat
          <PrimaryKbd>⏎</PrimaryKbd>
        </PrimaryButton>
      </div>
    </form>
  );
};

/** The manager-side dashboard: what every project's manager is doing, what needs you, and the token heatmap. */
export const DashboardView: React.FC<DashboardViewProps> = ({
  jobs,
  projects = [],
  onOpenProject,
  onRefresh,
  serviceOnline = true,
}) => {
  const { t } = useTranslation("dashboard");
  const analytics = useDashboardAnalytics();
  const { activity } = analytics;
  const managers = useManagerActivity();

  const [, tick] = React.useReducer((n: number) => n + 1, 0);
  React.useEffect(() => {
    const timer = window.setInterval(tick, 30_000);
    return () => window.clearInterval(timer);
  }, []);

  const [missions, setMissions] = React.useState<Mission[]>([]);
  React.useEffect(() => {
    let live = true;
    const load = () =>
      bridge
        .listMissions()
        .then((list) => live && setMissions(list))
        .catch(() => {});
    void load();
    const timer = window.setInterval(load, 15_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, []);

  const rows = React.useMemo(
    () =>
      projects
        .map((project) => ({ project, status: projectStatus(project.name, missions, jobs, managers[project.name]?.working ?? false) }))
        .sort(
          (a, b) =>
            Number(b.status.busy) - Number(a.status.busy) ||
            (b.status.updated ?? "").localeCompare(a.status.updated ?? "") ||
            a.project.name.localeCompare(b.project.name),
        ),
    [projects, missions, jobs, managers],
  );

  const working = rows.filter((r) => r.status.busy).length;

  const waiting = rows.flatMap(({ project, status }) =>
    status.missions
      .filter((m) => m.state === "AwaitingApproval" || m.state === "Paused" || m.state === "Review")
      .map((m) => ({ project: project.name, mission: m })),
  );

  const activityFeed = rows
    .flatMap(({ project, status }) =>
      status.missions.flatMap((m) => (m.log ?? []).map((entry) => ({ ...entry, project: project.name, mission: m.title, missionId: m.id }))),
    )
    .sort((a, b) => b.at.localeCompare(a.at))
    .slice(0, 14);

  const summary = serviceOnline
    ? `${working} of ${projects.length} managers working · ${waiting.length} need you`
    : t("header.systemsDown");

  return (
    <div
      data-testid="dashboard-view"
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-6 pt-5 max-md:px-3.5 max-md:pt-4"
    >
      <header className="flex shrink-0 items-end gap-3">
        <div className="flex min-w-0 flex-col gap-1.5">
          <h1 className="m-0 text-[26px] font-semibold tracking-[-0.025em]">{buildGreeting(new Date(), t)}</h1>
          <p className="m-0 flex items-center gap-2 text-[13px] text-muted-foreground">
            <span className={`size-1.5 rounded-full ${serviceOnline ? "bg-success" : "bg-destructive"}`} aria-hidden="true" />
            {summary}
          </p>
        </div>
        {onRefresh && (
          <GhostButton className="ml-auto" onClick={onRefresh}>
            <RefreshCw className="size-3.5" aria-hidden="true" />
            {t("command.refresh")}
          </GhostButton>
        )}
      </header>

      <AskManager projects={projects} onSubmit={(project, text) => onOpenProject?.(project, { draft: text || undefined })} />

      <div className="grid shrink-0 grid-cols-1 gap-3.5 xl:grid-cols-12">
        <Card
          className="min-h-[300px] xl:col-span-8"
          title="Managers"
          meta={<Pill tone={working ? "ok" : "mute"} dot live={working > 0}>{working} working</Pill>}
          bodyClassName="divide-y divide-border/60"
        >
          {rows.length === 0 && (
            <div className="px-4 py-10 text-center text-[12.5px] text-muted-foreground">No projects yet.</div>
          )}
          {rows.map(({ project, status }) => (
            <button
              key={project.name}
              type="button"
              onClick={() => onOpenProject?.(project.name)}
              className="grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.6fr)_auto] items-center gap-4 px-4 py-3 text-left hover:bg-secondary/40 max-md:grid-cols-1 max-md:gap-2"
            >
              <div className="flex min-w-0 items-center gap-2.5">
                <span className={status.busy ? "text-success" : "text-muted-foreground"}><Dot live={status.busy} /></span>
                <div className="min-w-0">
                  <div className="truncate text-[13.5px] font-medium text-foreground">{project.name}</div>
                  <div className="truncate font-mono text-[10.5px] text-muted-foreground">
                    {status.live.length} missions · {status.taskCount} tasks · ${status.cost.toFixed(2)}
                  </div>
                </div>
              </div>
              <div className="flex min-w-0 flex-col gap-1.5">
                <span className={cn("truncate text-[12.5px]", status.currentTask ? "text-foreground" : "text-muted-foreground")}>
                  {status.currentTask ?? status.headline}
                </span>
                <Strip status={status} />
              </div>
              <div className="flex items-center gap-2 max-md:justify-between">
                {status.attention.length > 0 ? <Pill tone="warn">Needs you</Pill> : null}
                <span className="w-[72px] text-right font-mono text-[10.5px] text-muted-foreground">{ago(status.updated)}</span>
              </div>
            </button>
          ))}
        </Card>

        <Card
          className="min-h-[300px] xl:col-span-4"
          title="Needs you"
          meta={waiting.length ? <Pill tone="warn">{waiting.length}</Pill> : undefined}
          bodyClassName="divide-y divide-border/60 overflow-y-auto"
        >
          {waiting.length === 0 && (
            <div className="px-4 py-10 text-center text-[12.5px] text-muted-foreground">
              Nothing is waiting on you. The managers have it.
            </div>
          )}
          {waiting.map(({ project, mission }) => (
            <div key={mission.id} className="flex flex-col gap-2 px-4 py-3">
              <button
                type="button"
                onClick={() => onOpenProject?.(project, { missionId: mission.id })}
                className="flex min-w-0 flex-col gap-0.5 text-left"
              >
                <span className="truncate text-[13px] font-medium text-foreground">{mission.title}</span>
                <span className="truncate font-mono text-[10.5px] text-muted-foreground">
                  {project} · {mission.state === "AwaitingApproval" ? "awaiting approval" : mission.state === "Paused" ? mission.pauseReason || "paused" : "ready for review"}
                </span>
              </button>
              <div className="flex gap-2">
                {mission.state === "AwaitingApproval" && (
                  <PrimaryButton size="sm" onClick={() => void bridge.missionAction(mission.id, "approve")}>Approve</PrimaryButton>
                )}
                {mission.state === "Paused" && (
                  <GhostButton size="sm" onClick={() => void bridge.missionAction(mission.id, "resume")}>Resume</GhostButton>
                )}
                <GhostButton size="sm" onClick={() => onOpenProject?.(project, { missionId: mission.id })}>
                  Open <ArrowUp className="size-3 rotate-45" aria-hidden="true" />
                </GhostButton>
              </div>
            </div>
          ))}
        </Card>
      </div>

      <div className="grid min-h-[300px] flex-1 grid-cols-1 gap-3.5 xl:grid-cols-12 max-md:flex-none max-md:[&>*]:min-h-[280px]">
        <HeatmapCard className="xl:col-span-7" activity={activity} weeks={26} title={t("command.heat.title")} />
        <Card className="xl:col-span-5" title="Manager activity" bodyClassName="overflow-y-auto px-4 py-2">
          {activityFeed.length === 0 && (
            <div className="py-10 text-center text-[12.5px] text-muted-foreground">No activity yet.</div>
          )}
          <ol className="m-0 flex list-none flex-col p-0">
            {activityFeed.map((entry, i) => (
              <li key={`${entry.at}-${i}`}>
                <button
                  type="button"
                  onClick={() => onOpenProject?.(entry.project, { missionId: entry.missionId })}
                  className="relative flex w-full gap-3 py-2 pl-4 text-left text-[12.5px] hover:bg-secondary/30"
                >
                  <span className="absolute left-0 top-[15px] size-1.5 rounded-full bg-border" aria-hidden="true" />
                  <span className="min-w-0 flex-1">
                    <span className="block text-foreground">{entry.message}</span>
                    <span className="block truncate font-mono text-[10.5px] text-muted-foreground">{entry.project} · {entry.mission}</span>
                  </span>
                  <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{ago(entry.at)}</span>
                </button>
              </li>
            ))}
          </ol>
        </Card>
      </div>
    </div>
  );
};
