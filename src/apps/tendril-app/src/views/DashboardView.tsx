import React from "react";
import { RefreshCw } from "lucide-react";
import { useTranslation, type TFunction } from "../i18n";
import type { Job, Mission, PlanSummary, ProjectSummary } from "../types/api";
import { bridge } from "../api/bridge";
import { projectStatus } from "../utils/projectStatus";
import { useRecentProjects } from "../state/recentProjects";
import { useManagerActivity } from "../state/managerActivity";
import { GhostButton, PrimaryButton, PrimaryKbd } from "../components/page/kit";
import { InsightsSections } from "./InsightsView";

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
      className="flex shrink-0 flex-col gap-3 rounded-xl border border-[#1f2a44] bg-card py-3.5 pl-4 pr-3.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_0_0_4px_rgba(27,146,255,0.035),0_18px_40px_-24px_rgba(27,146,255,0.35)]"
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
  onSelectPlan,
  projects = [],
  onOpenProject,
  onRefresh,
  serviceOnline = true,
}) => {
  const { t } = useTranslation("dashboard");
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

  const statuses = React.useMemo(
    () => projects.map((project) => projectStatus(project.name, missions, jobs, managers[project.name]?.working ?? false)),
    [projects, missions, jobs, managers],
  );
  const working = statuses.filter((st) => st.busy).length;
  // The sidebar's project dots carry who needs you; the line here just counts it.
  const needYou =
    statuses.filter((st) => st.attention.length > 0).length +
    projects.filter((p) => managers[p.name]?.asked).length;

  const summary = serviceOnline
    ? `${working} of ${projects.length} managers working · ${needYou} need you`
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

      <InsightsSections onSelectPlan={onSelectPlan} />
    </div>
  );
};
