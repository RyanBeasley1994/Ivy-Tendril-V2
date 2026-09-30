import React from "react";
import {
  Check,
  ChevronDown,
  CircleX,
  Rocket,
  ShieldCheck,
  Clock,
  Eye,
  FileText,
  GitBranch,
  GitMerge,
  GitPullRequest,
  OctagonPause,
  Sparkles,
  Zap,
} from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import type { DashboardActivity, Job, ProjectSummary } from "../../types/api";
import {
  AgentAvatar,
  Card,
  GhostButton,
  HEAT,
  Kbd,
  Pill,
  PrimaryButton,
  PrimaryKbd,
  Seg,
  Spark,
  heatLevel,
  type Tone,
} from "../../components/page/kit";
import {
  FLEET_WINDOW_MINUTES,
  buildHeatDays,
  fleetAxis,
  formatDuration,
  lastDays,
  summarizeHeat,
  type ActivityEvent,
  type Decision,
  type FleetLane,
  type FleetStats,
  type HeatMetric,
  type SegmentKind,
} from "../../utils/fleet";
import { formatAge, formatTokens } from "../../utils/commandCenter";
import { formatCurrency } from "../../utils/dashboardMetrics";

/* ------------------------------------------------------------------ composer */

export const CommandComposer: React.FC<{
  projects: ProjectSummary[];
  onSubmit: (description: string, project?: string) => void;
}> = ({ projects, onSubmit }) => {
  const { t } = useTranslation("dashboard");
  const [draft, setDraft] = React.useState("");
  const [project, setProject] = React.useState<string>("");
  const inputRef = React.useRef<HTMLInputElement>(null);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    onSubmit(draft.trim(), project || undefined);
    setDraft("");
  };

  return (
    <form
      onSubmit={submit}
      data-testid="command-composer"
      className="flex shrink-0 flex-col gap-3 rounded-xl border border-[#22303d] bg-card py-3.5 pl-4 pr-3.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_0_0_4px_rgba(0,204,146,0.035),0_18px_40px_-24px_rgba(0,204,146,0.35)]"
    >
      <div className="flex items-center gap-3">
        <Sparkles className="size-[18px] shrink-0 text-success" aria-hidden="true" />
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={t("command.composer.placeholder")}
          aria-label={t("command.composer.label")}
          className="min-w-0 flex-1 border-0 bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground"
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {projects.length > 0 && (
          <label className="relative inline-flex h-[26px] items-center gap-1.5 rounded-[7px] border border-border bg-muted pl-2 pr-6 text-xs text-foreground">
            <GitBranch className="size-3 text-muted-foreground" aria-hidden="true" />
            <span className="sr-only">{t("command.composer.project")}</span>
            <select
              value={project}
              onChange={(e) => setProject(e.target.value)}
              className="appearance-none bg-transparent pr-1 text-xs outline-none"
            >
              <option value="">{t("command.composer.anyProject")}</option>
              {projects.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
            <ChevronDown className="pointer-events-none absolute right-1.5 size-3 text-muted-foreground" aria-hidden="true" />
          </label>
        )}
        <span className="ml-1 hidden gap-3 font-mono text-[11px] text-muted-foreground md:flex">
          <span>{t("command.composer.hintDetails")}</span>
        </span>
        <PrimaryButton type="submit" className="ml-auto h-8">
          {t("command.composer.submit")}
          <PrimaryKbd>⏎</PrimaryKbd>
        </PrimaryButton>
      </div>
    </form>
  );
};

/* --------------------------------------------------------------------- fleet */

const SEGMENT_STYLE: Record<SegmentKind, React.CSSProperties> = {
  explore: { background: "#1b2733", boxShadow: "inset 0 0 0 1px #2a3847" },
  implement: {
    background: "linear-gradient(180deg,#19e0a5,#00b582)",
    boxShadow: "inset 0 1px 0 rgba(255,255,255,.25)",
  },
  ship: { background: "#4f86e6", boxShadow: "inset 0 1px 0 rgba(255,255,255,.2)" },
  failed: { background: "#e25c66" },
  stalled: {
    background:
      "repeating-linear-gradient(135deg,rgba(240,181,74,.55) 0 4px,rgba(240,181,74,.18) 4px 8px)",
  },
  done: { background: "#2a3644" },
};

const LANE_PILL: Record<FleetLane["state"], Tone> = {
  running: "ok",
  stalled: "warn",
  queued: "mute",
  failed: "bad",
  done: "mute",
};

export const FleetCard: React.FC<{
  lanes: FleetLane[];
  stats: FleetStats;
  jobLabel: (job: Job) => string;
  onOpenJob: (jobId: string) => void;
  onStopAll?: () => void;
  className?: string;
}> = ({ lanes, stats, jobLabel, onOpenJob, onStopAll, className }) => {
  const { t } = useTranslation("dashboard");
  const axis = fleetAxis();
  const grid = "grid grid-cols-[minmax(0,230px)_minmax(0,1fr)_72px] gap-3.5";
  const legend: [SegmentKind, string][] = [
    ["explore", t("command.fleet.legend.explore")],
    ["implement", t("command.fleet.legend.implement")],
    ["ship", t("command.fleet.legend.ship")],
    ["failed", t("command.fleet.legend.failed")],
    ["stalled", t("command.fleet.legend.stalled")],
  ];

  return (
    <Card
      testId="fleet-card"
      className={className}
      title={t("command.fleet.title")}
      meta={
        <>
          {stats.running > 0 && (
            <Pill tone="ok" dot live>
              {t("command.fleet.live")}
            </Pill>
          )}
          <span className="ml-1 text-xs text-muted-foreground">
            {t("command.fleet.window", { minutes: String(FLEET_WINDOW_MINUTES) })}
          </span>
        </>
      }
      actions={
        onStopAll &&
        stats.running + stats.queued > 0 && (
          <GhostButton size="sm" onClick={onStopAll}>
            <OctagonPause className="size-3.5" aria-hidden="true" />
            {t("command.fleet.stopAll")}
          </GhostButton>
        )
      }
    >
      <div className={cn(grid, "px-4 pb-1.5 pt-3 font-mono text-[10.5px] text-muted-foreground")}>
        <span>{t("command.fleet.columns.job")}</span>
        <div className="flex justify-between">
          {axis.map((label) => (
            <span key={label}>{label}</span>
          ))}
          <span className="text-success">{t("command.fleet.now")}</span>
        </div>
        <span className="text-right">{t("command.fleet.columns.tokens")}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {lanes.length === 0 && (
          <div className="flex h-full min-h-[120px] items-center justify-center border-t border-border/70 text-[13px] text-muted-foreground">
            {t("command.fleet.empty")}
          </div>
        )}
        {lanes.map((lane) => (
          <button
            key={lane.job.id}
            type="button"
            onClick={() => onOpenJob(lane.job.id)}
            className={cn(
              grid,
              "h-10 w-full items-center border-t border-border/70 px-4 text-left transition-colors hover:bg-muted/60",
            )}
          >
            <span className="flex min-w-0 items-center gap-2.5">
              <AgentAvatar agent={lane.job.model} />
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="truncate text-[12.5px]">{jobLabel(lane.job)}</span>
                <span className="flex items-center gap-1.5">
                  {lane.job.planId && (
                    <span className="font-mono text-[10.5px] text-muted-foreground">#{lane.job.planId}</span>
                  )}
                  <Pill tone={LANE_PILL[lane.state]} className="h-4 px-1.5 text-[10px]">
                    {lane.state === "stalled"
                      ? t("command.fleet.stalledFor", { minutes: String(lane.stalledFor ?? 0) })
                      : t(`command.fleet.state.${lane.state}`)}
                  </Pill>
                </span>
              </span>
            </span>
            <span
              className="relative h-10"
              style={{
                backgroundImage: "linear-gradient(90deg,var(--border) 1px,transparent 1px)",
                backgroundSize: "25% 100%",
              }}
            >
              {lane.segments.map((seg, i) => (
                <span
                  key={i}
                  className="absolute bottom-[9px] top-[9px] rounded-[5px]"
                  style={{
                    left: `${seg.from}%`,
                    width: `max(3px, calc(${seg.to - seg.from}% - 2px))`,
                    ...SEGMENT_STYLE[seg.kind],
                  }}
                />
              ))}
            </span>
            <span className="text-right font-mono text-xs">{formatTokens(lane.job.tokens) ?? "—"}</span>
          </button>
        ))}
      </div>
      <div className="grid shrink-0 grid-cols-4 border-t border-border/70">
        {[
          [t("command.fleet.stats.running"), String(stats.running)],
          [t("command.fleet.stats.tokens"), formatTokens(stats.tokensInFlight) ?? "0"],
          [t("command.fleet.stats.spend"), formatCurrency(stats.spendInFlight)],
          [
            t("command.fleet.stats.median"),
            stats.medianSeconds != null ? formatDuration(stats.medianSeconds) : "—",
          ],
        ].map(([label, value], i) => (
          <div key={label} className={cn("flex flex-col gap-1 px-4 py-3", i > 0 && "border-l border-border/70")}>
            <span className="text-[11.5px] text-muted-foreground">{label}</span>
            <span className="text-[17px] font-semibold tracking-[-0.01em]">{value}</span>
          </div>
        ))}
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-3.5 px-4 pb-2.5 pt-2 font-mono text-[10.5px] text-muted-foreground">
        {legend.map(([kind, label]) => (
          <span key={kind} className="flex items-center gap-1.5">
            <span className="h-2 w-3.5 rounded-[3px]" style={SEGMENT_STYLE[kind]} />
            {label}
          </span>
        ))}
        {stats.queued > 0 && (
          <span className="ml-auto">{t("command.fleet.queued", { count: stats.queued })}</span>
        )}
      </div>
    </Card>
  );
};

/* ----------------------------------------------------------------- decisions */

const DECISION_STYLE: Record<
  Decision["kind"],
  { icon: React.ComponentType<{ className?: string }>; tile: string }
> = {
  missionApproval: { icon: ShieldCheck, tile: "bg-warning/13 text-warning" },
  missionReview: { icon: Rocket, tile: "bg-info/14 text-info" },
  review: { icon: Eye, tile: "bg-info/14 text-info" },
  failed: { icon: CircleX, tile: "bg-destructive/14 text-destructive" },
  stalled: { icon: Clock, tile: "bg-warning/13 text-warning" },
  draft: { icon: FileText, tile: "bg-primary/12 text-success" },
};

export const DecisionsCard: React.FC<{
  items: Decision[];
  total: number;
  onOpen: (target: string) => void;
  onOpenAll: () => void;
  className?: string;
}> = ({ items, total, onOpen, onOpenAll, className }) => {
  const { t } = useTranslation("dashboard");
  const [selected, setSelected] = React.useState(0);
  const listRef = React.useRef<HTMLDivElement>(null);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "j" || e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((i) => Math.min(items.length - 1, i + 1));
    } else if (e.key === "k" || e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter" && items[selected]) {
      e.preventDefault();
      onOpen(items[selected].target);
    }
  };

  const meta = (d: Decision): string => {
    const parts: string[] = [];
    if (d.planId) parts.push(`#${d.planId}`);
    if (d.checks) parts.push(t("command.decisions.checks", { passed: String(d.checks.passed), total: String(d.checks.total) }));
    if (d.level) parts.push(d.level);
    if (d.waitingMs != null) parts.push(formatAge(d.waitingMs));
    return parts.join(" · ");
  };

  return (
    <Card
      testId="decisions-card"
      className={className}
      title={t("command.decisions.title")}
      meta={total > 0 ? <Pill>{total}</Pill> : undefined}
      actions={
        <button type="button" onClick={onOpenAll} className="text-xs text-success hover:text-foreground">
          {t("command.decisions.viewAll")}
        </button>
      }
    >
      {items.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 text-[13px] text-muted-foreground">
          <Check className="size-5 text-success" aria-hidden="true" />
          {t("command.decisions.empty")}
        </div>
      ) : (
        <div
          ref={listRef}
          tabIndex={0}
          onKeyDown={onKeyDown}
          aria-label={t("command.decisions.title")}
          className="min-h-0 flex-1 overflow-y-auto outline-none"
        >
          {items.map((d, i) => {
            const style = DECISION_STYLE[d.kind];
            const Icon = style.icon;
            const isSel = i === selected;
            const action = t(`command.decisions.action.${d.kind}`);
            return (
              <div
                key={d.target + d.kind}
                onMouseEnter={() => setSelected(i)}
                className={cn(
                  "flex gap-3 border-t border-border/70 px-4 first:border-t-0",
                  isSel ? "items-start bg-primary/[0.05] py-3 shadow-[inset_2px_0_0_var(--primary)]" : "items-center py-3",
                )}
              >
                <span className={cn("flex size-[30px] shrink-0 items-center justify-center rounded-[9px]", style.tile)}>
                  <Icon className="size-[15px]" />
                </span>
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <span className="text-[13px] font-medium">{t(`command.decisions.kind.${d.kind}`)}</span>
                  <span className="truncate text-xs text-muted-foreground">{d.title}</span>
                  {isSel && <span className="font-mono text-[10.5px] text-muted-foreground">{meta(d)}</span>}
                  {isSel && (
                    <div className="mt-1.5 flex gap-1.5">
                      <PrimaryButton size="sm" onClick={() => onOpen(d.target)}>
                        {action}
                      </PrimaryButton>
                      {d.planTarget && (
                        <GhostButton size="sm" onClick={() => onOpen(d.planTarget!)}>
                          {t("command.decisions.viewPlan")}
                        </GhostButton>
                      )}
                    </div>
                  )}
                </div>
                {!isSel && (
                  <div className="flex shrink-0 gap-1.5">
                    {d.planTarget && (
                      <GhostButton size="sm" onClick={() => onOpen(d.planTarget!)}>
                        {t("command.decisions.viewPlan")}
                      </GhostButton>
                    )}
                    <GhostButton size="sm" onClick={() => onOpen(d.target)}>
                      {action}
                    </GhostButton>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      <div className="flex shrink-0 items-center gap-3 border-t border-border/70 px-4 py-2.5 font-mono text-[10.5px] text-muted-foreground">
        <span className="flex items-center gap-1">
          <Kbd>J</Kbd>
          <Kbd>K</Kbd>
          {t("command.decisions.keys.move")}
        </span>
        <span className="flex items-center gap-1">
          <Kbd>⏎</Kbd>
          {t("command.decisions.keys.open")}
        </span>
      </div>
    </Card>
  );
};

/* --------------------------------------------------------------------- today */

export const TodayCard: React.FC<{
  activity: DashboardActivity | null;
  shippedToday: number;
  shippedSeries: number[];
  className?: string;
}> = ({ activity, shippedToday, shippedSeries, className }) => {
  const { t } = useTranslation("dashboard");
  const tokens = lastDays(activity, "tokens", 8);
  const cost = lastDays(activity, "cost", 8);
  const prior = (series: number[]) => {
    const p = series.slice(0, -1).filter((v) => v > 0);
    return p.length ? p.reduce((a, b) => a + b, 0) / p.length : 0;
  };
  const delta = (series: number[]) => {
    const avg = prior(series);
    if (avg <= 0) return null;
    return Math.round(((series[series.length - 1] - avg) / avg) * 100);
  };
  const monthStart = new Date().toISOString().slice(0, 8) + "01";
  const monthToDate = (activity?.dailyCosts ?? [])
    .filter((d) => d.date >= monthStart)
    .reduce((sum, d) => sum + d.cost, 0);
  const forecast = activity?.forecast.calendarProjection ?? null;

  const rows: { label: string; value: string; note: React.ReactNode; spark: number[]; color: string }[] = [
    {
      label: t("command.today.shipped"),
      value: String(shippedToday),
      note: <Pill tone="mute">{t("command.today.last8")}</Pill>,
      spark: shippedSeries,
      color: "var(--success)",
    },
    {
      label: t("command.today.tokens"),
      value: formatTokens(tokens[tokens.length - 1]) ?? "0",
      note: (() => {
        const d = delta(tokens);
        return d == null ? null : (
          <Pill tone={d >= 0 ? "ok" : "info"}>{t("command.today.vsAvg", { delta: `${d > 0 ? "+" : ""}${d}%` })}</Pill>
        );
      })(),
      spark: tokens,
      color: "var(--info)",
    },
    {
      label: t("command.today.spend"),
      value: formatCurrency(cost[cost.length - 1]),
      note: (
        <Pill tone="mute">
          {forecast != null
            ? t("command.today.monthForecast", { spent: formatCurrency(monthToDate), forecast: formatCurrency(forecast) })
            : t("command.today.month", { spent: formatCurrency(monthToDate) })}
        </Pill>
      ),
      spark: cost,
      color: "var(--violet)",
    },
  ];

  return (
    <Card testId="today-card" className={className} title={t("command.today.title")}>
      {rows.map((row, i) => (
        <div
          key={row.label}
          className={cn("flex flex-1 items-center gap-2.5 px-4 py-3", i > 0 && "border-t border-border/70")}
        >
          <div className="flex min-w-0 flex-col gap-1.5">
            <span className="text-[11.5px] text-muted-foreground">{row.label}</span>
            <span className="text-[22px] font-semibold leading-none tracking-[-0.02em]">{row.value}</span>
            {row.note && <span className="flex">{row.note}</span>}
          </div>
          <span className="ml-auto">
            <Spark values={row.spark} color={row.color} />
          </span>
        </div>
      ))}
    </Card>
  );
};

/* ------------------------------------------------------------------- heatmap */

const WEEKDAY_FMT = new Intl.DateTimeFormat(undefined, { weekday: "long" });
const MONTH_FMT = new Intl.DateTimeFormat(undefined, { month: "short" });
const DAY_FMT = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

const formatHeat = (metric: HeatMetric, value: number): string =>
  metric === "tokens"
    ? (formatTokens(value) ?? "0")
    : metric === "cost"
      ? formatCurrency(value)
      : String(Math.round(value));

export const HeatmapCard: React.FC<{
  activity: DashboardActivity | null;
  weeks: number;
  cell?: number;
  title: string;
  className?: string;
  footer?: boolean;
}> = ({ activity, weeks, cell = 12, title, className, footer = true }) => {
  const { t } = useTranslation("dashboard");
  const [metric, setMetric] = React.useState<HeatMetric>("tokens");
  const days = buildHeatDays(activity, metric, weeks);
  const summary = summarizeHeat(days);
  const max = Math.max(0, ...days.map((d) => d.value));
  const gap = cell >= 12 ? 3 : 2.5;
  const todayIndex = days.reduce((last, d, i) => (d.value >= 0 ? i : last), -1);

  // Month labels at the first week each month starts in.
  const months: { col: number; label: string }[] = [];
  for (let w = 0; w < weeks; w++) {
    const first = days[w * 7];
    const d = new Date(first.date + "T00:00:00");
    if (d.getDate() <= 7 && (months.length === 0 || months[months.length - 1].col < w - 2)) {
      months.push({ col: w, label: MONTH_FMT.format(d) });
    }
  }

  return (
    <Card
      testId="heatmap-card"
      className={className}
      title={title}
      actions={
        <Seg<HeatMetric>
          label={title}
          value={metric}
          onChange={setMetric}
          options={[
            { value: "tokens", label: t("command.heat.tokens") },
            { value: "plans", label: t("command.heat.plans") },
            { value: "cost", label: t("command.heat.spend") },
          ]}
        />
      }
    >
      <div className="flex flex-1 flex-col gap-3 px-4 py-3.5">
        <div className="flex items-baseline gap-2.5">
          <span className="text-[22px] font-semibold tracking-[-0.02em]">{formatHeat(metric, summary.total)}</span>
          <span className="text-xs text-muted-foreground">{t("command.heat.lastWeeks", { weeks: String(weeks) })}</span>
          {summary.streak > 1 && (
            <Pill tone="ok" className="ml-auto">
              <Zap className="size-3" aria-hidden="true" />
              {t("command.heat.streak", { days: String(summary.streak) })}
            </Pill>
          )}
        </div>
        <div className="flex gap-2 overflow-hidden">
          <div
            className="grid shrink-0 font-mono text-[9px] text-muted-foreground"
            style={{ gridTemplateRows: `repeat(7, ${cell}px)`, gap, lineHeight: `${cell}px` }}
            aria-hidden="true"
          >
            {["", "Mon", "", "Wed", "", "Fri", ""].map((l, i) => (
              <span key={i}>{l}</span>
            ))}
          </div>
          <div className="flex flex-col gap-1.5">
            <div
              role="img"
              aria-label={title}
              className="grid grid-flow-col"
              style={{ gridTemplateRows: `repeat(7, ${cell}px)`, gridAutoColumns: `${cell}px`, gap }}
            >
              {days.map((d, i) => (
                <span
                  key={d.date}
                  title={d.value >= 0 ? `${DAY_FMT.format(new Date(d.date + "T00:00:00"))} · ${formatHeat(metric, d.value)}` : undefined}
                  style={{
                    borderRadius: cell >= 12 ? 3 : 2,
                    background: d.value < 0 ? "transparent" : HEAT[heatLevel(d.value, max)],
                    boxShadow: i === todayIndex ? "0 0 0 1.5px var(--foreground)" : undefined,
                  }}
                />
              ))}
            </div>
            <div className="relative h-3 font-mono text-[10px] text-muted-foreground" aria-hidden="true">
              {months.map((m) => (
                <span key={m.col} className="absolute" style={{ left: m.col * (cell + gap) }}>
                  {m.label}
                </span>
              ))}
            </div>
          </div>
        </div>
        {footer && (
          <div className="mt-auto grid grid-cols-3 gap-2 border-t border-border/70 pt-2.5 text-[11.5px] text-muted-foreground">
            <span className="flex flex-col gap-0.5">
              {t("command.heat.avg")}
              <span className="font-mono text-[13px] text-foreground">{formatHeat(metric, summary.avg)}</span>
            </span>
            <span className="flex flex-col gap-0.5">
              {t("command.heat.peak")}
              <span className="font-mono text-[13px] text-foreground">
                {summary.peak
                  ? `${formatHeat(metric, summary.peak.value)} · ${DAY_FMT.format(new Date(summary.peak.date + "T00:00:00"))}`
                  : "—"}
              </span>
            </span>
            <span className="flex flex-col gap-0.5">
              {t("command.heat.busiest")}
              <span className="font-mono text-[13px] text-foreground">
                {summary.busiestWeekday != null
                  ? WEEKDAY_FMT.format(new Date(2024, 0, 7 + summary.busiestWeekday))
                  : "—"}
              </span>
            </span>
          </div>
        )}
      </div>
    </Card>
  );
};

/* ------------------------------------------------------------------ activity */

const ACTIVITY_STYLE: Record<ActivityEvent["kind"], { icon: React.ComponentType<{ className?: string }>; color: string }> = {
  merged: { icon: GitMerge, color: "text-violet" },
  finished: { icon: Check, color: "text-success" },
  failed: { icon: CircleX, color: "text-destructive" },
  drafted: { icon: Sparkles, color: "text-success" },
  pr: { icon: GitPullRequest, color: "text-info" },
};

export const ActivityCard: React.FC<{
  events: ActivityEvent[];
  onOpen: (target: string) => void;
  className?: string;
}> = ({ events, onOpen, className }) => {
  const { t } = useTranslation("dashboard");
  const now = Date.now();
  return (
    <Card testId="activity-card" className={className} title={t("command.activity.title")}>
      {events.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-[13px] text-muted-foreground">
          {t("command.activity.empty")}
        </div>
      ) : (
        <div className="relative flex min-h-0 flex-1 flex-col overflow-y-auto px-4 py-1.5">
          <span className="absolute bottom-5 left-[30px] top-5 w-px bg-border" aria-hidden="true" />
          {events.map((ev, i) => {
            const style = ACTIVITY_STYLE[ev.kind];
            const Icon = style.icon;
            return (
              <button
                key={i}
                type="button"
                onClick={() => ev.target && onOpen(ev.target)}
                className="relative flex items-start gap-3 rounded-md py-2 text-left hover:bg-muted/40"
              >
                <span
                  className={cn(
                    "flex size-7 shrink-0 items-center justify-center rounded-full border border-border bg-card",
                    style.color,
                  )}
                >
                  <Icon className="size-[13px]" />
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate text-[12.5px]">
                    {t(`command.activity.kind.${ev.kind}`)}
                    {ev.planId && <span className="ml-1 font-mono text-xs text-foreground">#{ev.planId}</span>}
                  </span>
                  <span className="truncate text-[11px] text-muted-foreground">{ev.title}</span>
                </span>
                <span className="pt-0.5 font-mono text-[10.5px] text-muted-foreground">{formatAge(now - ev.at)}</span>
              </button>
            );
          })}
        </div>
      )}
    </Card>
  );
};

