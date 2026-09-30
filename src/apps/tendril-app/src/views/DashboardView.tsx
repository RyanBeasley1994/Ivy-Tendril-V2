import React from "react";
import { RefreshCw } from "lucide-react";
import { useTranslation, type TFunction } from "../i18n";
import { useEnumLabels } from "../i18n/enumLabels";
import type { Job, PlanSummary, ProjectSummary } from "../types/api";
import { useDashboardAnalytics } from "../hooks/useDashboardAnalytics";
import { toIsoDate, todayDayNumber } from "../utils/rollingAverage";
import { buildActivity, buildDecisions, buildFleet, buildFleetStats } from "../utils/fleet";
import { GhostButton } from "../components/page/kit";
import {
  ActivityCard,
  CommandComposer,
  DecisionsCard,
  FleetCard,
  HeatmapCard,
  TodayCard,
} from "./command/panels";

interface DashboardViewProps {
  plans: PlanSummary[];
  jobs: Job[];
  projects?: ProjectSummary[];
  onSelectJob?: (jobId: string) => void;
  onSelectPlan?: (planId: string) => void;
  /** Nav id from ShellLayout's nav items: "plans" | "review" | "jobs". */
  onNavigate?: (navId: string) => void;
  /** Opens the New Plan dialog, prefilled with what was typed into the composer. */
  onNewPlan?: (description?: string, project?: string) => void;
  /** Opens the confirm-and-stop-all dialog. */
  onStopAll?: () => void;
  /** Re-reads plans and jobs from the daemon. */
  onRefresh?: () => void;
  /** False while the service is unreachable. */
  serviceOnline?: boolean;
}

const buildGreeting = (now: Date, t: TFunction<"dashboard">): string => {
  const hour = now.getHours();
  if (hour >= 5 && hour < 12) return t("command.greeting.morning");
  if (hour >= 12 && hour < 17) return t("command.greeting.afternoon");
  return t("command.greeting.evening");
};

/** The V2 command center: composer, the agent fleet, what waits on you, and what today added up to. */
export const DashboardView: React.FC<DashboardViewProps> = ({
  plans,
  jobs,
  projects = [],
  onSelectJob,
  onSelectPlan,
  onNavigate,
  onNewPlan,
  onStopAll,
  onRefresh,
  serviceOnline = true,
}) => {
  const { t } = useTranslation("dashboard");
  const enumLabels = useEnumLabels();
  const analytics = useDashboardAnalytics();
  const { activity } = analytics;

  // Re-render every 30s so the fleet's "now" edge and the ages keep moving between polls.
  const [, tick] = React.useReducer((n: number) => n + 1, 0);
  React.useEffect(() => {
    const timer = window.setInterval(tick, 30_000);
    return () => window.clearInterval(timer);
  }, []);

  const now = Date.now();
  const lanes = buildFleet(jobs, now);
  const stats = buildFleetStats(jobs, now);
  const decisions = buildDecisions(plans, jobs, now);
  const events = buildActivity(jobs, analytics.mergedPrs);
  const stalled = lanes.filter((l) => l.state === "stalled").length;

  const today = todayDayNumber();
  const shipped = new Map(analytics.shippedFeatures.map((d) => [d.date, d.count]));
  const shippedSeries = Array.from({ length: 8 }, (_, i) => shipped.get(toIsoDate(today - 7 + i)) ?? 0);

  const open = (target: string) => {
    if (target.startsWith("plan:")) onSelectPlan?.(target.slice(5));
    else if (target.startsWith("job:")) onSelectJob?.(target.slice(4));
  };

  const jobLabel = (j: Job) => j.planTitle || j.prompt || (j.type && enumLabels.jobType(j.type)) || j.project;

  const summary = serviceOnline
    ? t("command.summary", {
        decisions: String(decisions.total),
        running: String(stats.running),
        queued: String(stats.queued),
      })
    : t("header.systemsDown");

  return (
    <div
      data-testid="dashboard-view"
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-6 pt-5"
    >
      <header className="flex shrink-0 items-end gap-3">
        <div className="flex min-w-0 flex-col gap-1.5">
          <h1 className="m-0 text-[26px] font-semibold tracking-[-0.025em]">{buildGreeting(new Date(now), t)}</h1>
          <p className="m-0 flex items-center gap-2 text-[13px] text-muted-foreground">
            <span
              className={`size-1.5 rounded-full ${serviceOnline ? "bg-success" : "bg-destructive"}`}
              aria-hidden="true"
            />
            {summary}
            {stalled > 0 && (
              <span className="text-warning">{t("command.summaryStalled", { count: stalled })}</span>
            )}
          </p>
        </div>
        {onRefresh && (
          <GhostButton className="ml-auto" onClick={onRefresh}>
            <RefreshCw className="size-3.5" aria-hidden="true" />
            {t("command.refresh")}
          </GhostButton>
        )}
      </header>

      <CommandComposer
        projects={projects}
        onSubmit={(description, project) => onNewPlan?.(description || undefined, project)}
      />

      <div className="grid shrink-0 grid-cols-1 gap-3.5 xl:h-[388px] xl:grid-cols-12">
        <FleetCard
          className="min-h-[300px] xl:col-span-8"
          lanes={lanes}
          stats={stats}
          jobLabel={jobLabel}
          onOpenJob={(id) => onSelectJob?.(id)}
          onStopAll={onStopAll}
        />
        <DecisionsCard
          className="min-h-[300px] xl:col-span-4"
          items={decisions.items}
          total={decisions.total}
          onOpen={open}
          onOpenAll={() => onNavigate?.("review")}
        />
      </div>

      <div className="grid min-h-[300px] flex-1 grid-cols-1 gap-3.5 xl:grid-cols-12">
        <TodayCard
          className="xl:col-span-3"
          activity={activity}
          shippedToday={shippedSeries[shippedSeries.length - 1]}
          shippedSeries={shippedSeries}
        />
        <HeatmapCard
          className="xl:col-span-5"
          activity={activity}
          weeks={26}
          title={t("command.heat.title")}
        />
        <ActivityCard className="xl:col-span-4" events={events} onOpen={open} />
      </div>
    </div>
  );
};
