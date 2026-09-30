import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import type { AgentCostBreakdown, DashboardActivity, RecentPlanCost } from "../../types/api";
import {
  AGENT_FILL,
  AgentAvatar,
  Bar,
  Card,
  MiniBars,
  Pill,
  Seg,
  agentKind,
  type Tone,
} from "../../components/page/kit";
import { formatTokens } from "../../utils/commandCenter";
import { formatCurrency } from "../../utils/dashboardMetrics";
import { toIsoDate, todayDayNumber } from "../../utils/rollingAverage";

/* ------------------------------------------------------------------ KPI strip */

export interface InsightKpi {
  id: string;
  label: string;
  value: string;
  note?: string | null;
  tone?: Tone;
  hint?: string | null;
  series?: number[];
  color?: string;
  onSelect?: () => void;
}

export const KpiStrip: React.FC<{ items: InsightKpi[]; loading?: boolean }> = ({ items, loading }) => (
  <div
    data-testid="insights-stats"
    className="grid shrink-0 overflow-hidden rounded-xl border border-border bg-card"
    style={{ gridTemplateColumns: `repeat(${Math.max(1, items.length)}, minmax(0, 1fr))` }}
  >
    {items.map((kpi, i) => {
      const body = (
        <>
          <span className="flex items-center gap-1.5">
            <span className="truncate text-xs text-muted-foreground">{kpi.label}</span>
            {kpi.note && (
              <Pill tone={kpi.tone ?? "mute"} className="ml-auto h-[18px] text-[10.5px]">
                {kpi.note}
              </Pill>
            )}
          </span>
          <span className="flex items-end justify-between gap-2">
            <span className={cn("text-[22px] font-semibold leading-none tracking-[-0.02em]", loading && "opacity-40")}>
              {kpi.value}
            </span>
            {kpi.series && kpi.series.some((v) => v > 0) && (
              <MiniBars values={kpi.series} color={kpi.color} height={24} />
            )}
          </span>
          {kpi.hint && <span className="truncate text-[11px] text-muted-foreground">{kpi.hint}</span>}
        </>
      );
      const cls = cn("flex min-w-0 flex-col gap-2.5 px-4 py-3.5 text-left", i > 0 && "border-l border-border/70");
      return kpi.onSelect ? (
        <button key={kpi.id} type="button" onClick={kpi.onSelect} className={cn(cls, "transition-colors hover:bg-muted/60")}>
          {body}
        </button>
      ) : (
        <div key={kpi.id} className={cls}>
          {body}
        </div>
      );
    })}
  </div>
);

/* --------------------------------------------------------------- dot matrix */

type DotMetric = "tokens" | "cost" | "plans";

const DOT_ROWS = 15;
const DAY_FMT = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" });

export const DotMatrixCard: React.FC<{
  activity: DashboardActivity | null;
  days?: number;
  className?: string;
}> = ({ activity, days = 30, className }) => {
  const { t } = useTranslation("dashboard");
  const [metric, setMetric] = React.useState<DotMetric>("tokens");
  const today = todayDayNumber();
  const byDay = new Map<string, number>();
  if (metric === "plans") for (const d of activity?.dailyPlans ?? []) byDay.set(d.date, d.count);
  else for (const d of activity?.dailyCosts ?? []) byDay.set(d.date, metric === "tokens" ? d.tokens : d.cost);

  const series = Array.from({ length: days }, (_, i) => {
    const date = toIsoDate(today - days + 1 + i);
    return { date, value: byDay.get(date) ?? 0 };
  });
  const max = Math.max(0, ...series.map((s) => s.value));
  const nonZero = series.filter((s) => s.value > 0);
  const avg = nonZero.length ? series.reduce((a, s) => a + s.value, 0) / series.length : 0;
  const avgRow = max > 0 ? (avg / max) * DOT_ROWS : 0;
  const fmt = (v: number) =>
    metric === "tokens" ? (formatTokens(v) ?? "0") : metric === "cost" ? formatCurrency(v) : String(Math.round(v));
  const color = metric === "tokens" ? "#19e0a5" : metric === "cost" ? "var(--violet)" : "var(--info)";
  const dotPx = 7;
  const gapPx = 4;
  const height = DOT_ROWS * dotPx + (DOT_ROWS - 1) * gapPx;

  return (
    <Card
      testId="insights-dots"
      className={className}
      title={t("insights.v2.perDay")}
      meta={
        <span className="ml-2 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <span className="w-3 border-t-[1.5px] border-dashed border-warning" aria-hidden="true" />
          {t("insights.v2.dailyAvg", { value: fmt(avg) })}
        </span>
      }
      actions={
        <Seg<DotMetric>
          label={t("insights.v2.perDay")}
          value={metric}
          onChange={setMetric}
          options={[
            { value: "tokens", label: t("insights.tokens") },
            { value: "cost", label: t("insights.cost") },
            { value: "plans", label: t("insights.plans") },
          ]}
        />
      }
    >
      <div className="flex flex-1 flex-col gap-2 px-4 pb-2.5 pt-5">
        <div
          role="img"
          aria-label={t("insights.v2.perDay")}
          className="relative grid items-end"
          style={{ gridTemplateColumns: `repeat(${days}, minmax(0, 1fr))`, gap: gapPx, height }}
        >
          {avgRow > 0 && (
            <span
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-0 z-10 border-t-[1.5px] border-dashed border-warning/60"
              style={{ bottom: (avgRow / DOT_ROWS) * height }}
            />
          )}
          {series.map((s, i) => {
            const lit = max > 0 ? Math.round((s.value / max) * DOT_ROWS) : 0;
            const isToday = i === series.length - 1;
            return (
              <div
                key={s.date}
                title={`${DAY_FMT.format(new Date(s.date + "T00:00:00"))} · ${fmt(s.value)}`}
                className="flex flex-col-reverse items-center"
                style={{ gap: gapPx }}
              >
                {Array.from({ length: DOT_ROWS }, (_, r) => (
                  <span
                    key={r}
                    className="rounded-full"
                    style={{
                      width: dotPx,
                      height: dotPx,
                      background: r < Math.max(lit, s.value > 0 ? 1 : 0) ? (isToday ? "var(--foreground)" : color) : "#141b23",
                    }}
                  />
                ))}
              </div>
            );
          })}
        </div>
        <div
          className="grid font-mono text-[10px] text-muted-foreground"
          style={{ gridTemplateColumns: `repeat(${days}, minmax(0, 1fr))`, gap: gapPx }}
          aria-hidden="true"
        >
          {series.map((s, i) => (
            <span key={s.date} className={cn("whitespace-nowrap text-center", i === days - 1 && "text-foreground")}>
              {i === days - 1
                ? t("insights.v2.today")
                : i % 7 === 0
                  ? new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(new Date(s.date + "T00:00:00"))
                  : ""}
            </span>
          ))}
        </div>
      </div>
    </Card>
  );
};

/* -------------------------------------------------------------- month spend */

export const MonthSpendCard: React.FC<{
  activity: DashboardActivity | null;
  agents: AgentCostBreakdown[];
  className?: string;
  onOpenForecast?: () => void;
}> = ({ activity, agents, className, onOpenForecast }) => {
  const { t } = useTranslation("dashboard");
  const now = new Date();
  const year = now.getFullYear();
  const month = now.getMonth();
  const daysInMonth = new Date(year, month + 1, 0).getDate();
  const dayOfMonth = now.getDate();
  const prefix = `${year}-${String(month + 1).padStart(2, "0")}-`;
  const byDay = new Map((activity?.dailyCosts ?? []).map((d) => [d.date, d.cost]));
  let running = 0;
  const cumulative = Array.from({ length: dayOfMonth }, (_, i) => (running += byDay.get(prefix + String(i + 1).padStart(2, "0")) ?? 0));
  const spent = running;
  const forecast = activity?.forecast.calendarProjection ?? null;
  const top = Math.max(spent, forecast ?? 0, 1) * 1.08;

  const W = 320;
  const H = 110;
  const x = (day: number) => (day / daysInMonth) * W;
  const y = (v: number) => H - 4 - (v / top) * (H - 12);
  const line = cumulative.map((v, i) => `${x(i + 1).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const monthName = new Intl.DateTimeFormat(undefined, { month: "long" }).format(now);

  const sorted = [...agents].sort((a, b) => b.cost - a.cost);
  const total = sorted.reduce((s, a) => s + a.cost, 0);

  return (
    <Card
      testId="insights-spend"
      className={className}
      title={t("insights.v2.spend")}
      meta={<span className="text-xs text-muted-foreground">{monthName}</span>}
      actions={
        onOpenForecast && (
          <button type="button" onClick={onOpenForecast} className="text-xs text-success hover:text-foreground">
            {t("insights.v2.breakdown")}
          </button>
        )
      }
    >
      <div className="flex flex-1 flex-col gap-2.5 px-4 py-3.5">
        <div className="flex items-baseline gap-2">
          <span className="text-[26px] font-semibold tracking-[-0.02em]">{formatCurrency(spent)}</span>
          {forecast != null && (
            <span className="font-mono text-xs text-muted-foreground">
              {t("insights.v2.forecast", { value: formatCurrency(forecast) })}
            </span>
          )}
        </div>
        <svg viewBox={`0 0 ${W} ${H}`} className="h-[110px] w-full" role="img" aria-label={t("insights.v2.spend")}>
          {cumulative.length > 1 && (
            <>
              <polygon
                points={`${x(1)},${H} ${line} ${x(cumulative.length)},${H}`}
                fill="var(--violet)"
                opacity={0.12}
              />
              <polyline points={line} fill="none" stroke="var(--violet)" strokeWidth={2} strokeLinejoin="round" />
            </>
          )}
          {forecast != null && (
            <line
              x1={x(dayOfMonth)}
              y1={y(spent)}
              x2={W}
              y2={y(forecast)}
              stroke="var(--violet)"
              strokeWidth={1.5}
              strokeDasharray="4 4"
              opacity={0.7}
            />
          )}
          <circle cx={x(dayOfMonth)} cy={y(spent)} r={4} fill="var(--background)" stroke="var(--foreground)" strokeWidth={2} />
        </svg>
        {sorted.length > 0 && (
          <>
            <div className="flex h-1.5 gap-[3px]" aria-hidden="true">
              {sorted.map((a) => (
                <span
                  key={a.agent}
                  className="rounded-[3px]"
                  style={{ flexGrow: total > 0 ? a.cost / total : 1, background: AGENT_FILL[agentKind(a.agent)] }}
                />
              ))}
            </div>
            <div className="grid grid-cols-2 gap-x-3.5 gap-y-1.5 text-xs">
              {sorted.slice(0, 4).map((a) => (
                <span key={a.agent} className="flex items-center gap-2">
                  <span className="size-[7px] rounded-[2px]" style={{ background: AGENT_FILL[agentKind(a.agent)] }} />
                  <span className="truncate">{a.agent}</span>
                  <span className="ml-auto font-mono text-[11.5px] text-muted-foreground">{formatCurrency(a.cost)}</span>
                </span>
              ))}
            </div>
          </>
        )}
      </div>
    </Card>
  );
};

/* ---------------------------------------------------------- costliest plans */

export const CostliestPlansCard: React.FC<{
  plans: RecentPlanCost[];
  onOpenPlan: (planId: string) => void;
  className?: string;
}> = ({ plans, onOpenPlan, className }) => {
  const { t } = useTranslation("dashboard");
  const priced = plans.filter((p) => p.cost != null && p.cost > 0).sort((a, b) => (b.cost ?? 0) - (a.cost ?? 0));
  const top = priced.slice(0, 5);
  const max = top[0]?.cost ?? 0;
  const avg = priced.length ? priced.reduce((s, p) => s + (p.cost ?? 0), 0) / priced.length : null;
  return (
    <Card
      testId="insights-costliest"
      className={className}
      title={t("insights.v2.costliest")}
      meta={
        avg != null ? (
          <span className="text-xs text-muted-foreground">{t("insights.v2.avgPlan", { value: formatCurrency(avg) })}</span>
        ) : undefined
      }
    >
      {top.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-[13px] text-muted-foreground">{t("insights.v2.noPlans")}</div>
      ) : (
        <div className="flex flex-col gap-3 px-4 py-3.5">
          {top.map((p) => (
            <button
              key={p.planId}
              type="button"
              onClick={() => onOpenPlan(String(p.planId))}
              className="grid grid-cols-[minmax(0,1fr)_minmax(0,0.8fr)_64px] items-center gap-3 rounded-md text-left text-[12.5px] hover:bg-muted/40"
            >
              <span className="flex min-w-0 items-center gap-2">
                <span className="font-mono text-[10.5px] text-muted-foreground">#{p.planId}</span>
                <span className="truncate">{p.title}</span>
              </span>
              <span className="block h-3.5 overflow-hidden rounded bg-muted">
                <span
                  className="block h-full rounded"
                  style={{
                    width: `${max > 0 ? ((p.cost ?? 0) / max) * 100 : 0}%`,
                    background: "linear-gradient(90deg,#0b8a64,#19e0a5)",
                  }}
                />
              </span>
              <span className="text-right font-mono text-xs">{formatCurrency(p.cost ?? 0)}</span>
            </button>
          ))}
        </div>
      )}
    </Card>
  );
};

/* ------------------------------------------------------------------- agents */

export const AgentsCard: React.FC<{ agents: AgentCostBreakdown[]; className?: string }> = ({ agents, className }) => {
  const { t } = useTranslation("dashboard");
  const sorted = [...agents].sort((a, b) => b.cost - a.cost);
  const total = sorted.reduce((s, a) => s + a.cost, 0);
  const cols = "grid grid-cols-[minmax(0,1.6fr)_64px_84px_84px_84px_minmax(0,1fr)] gap-3.5";
  return (
    <Card testId="insights-agents" className={className} title={t("insights.agents")}>
      <div className={cn(cols, "px-4 pb-1.5 pt-2.5 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground")}>
        <span>{t("insights.agent")}</span>
        <span className="text-right">{t("insights.plans")}</span>
        <span className="text-right">{t("insights.tokensPerPlan")}</span>
        <span className="text-right">{t("insights.tokens")}</span>
        <span className="text-right">{t("insights.cost")}</span>
        <span>{t("insights.v2.share")}</span>
      </div>
      {sorted.length === 0 && (
        <div className="border-t border-border/70 px-4 py-6 text-[13px] text-muted-foreground">{t("insights.noAgents")}</div>
      )}
      {sorted.map((a) => {
        const share = total > 0 ? (a.cost / total) * 100 : 0;
        return (
          <div key={a.agent} className={cn(cols, "h-10 items-center border-t border-border/70 px-4 text-[13px]")}>
            <span className="flex min-w-0 items-center gap-2.5">
              <AgentAvatar agent={a.agent} size={26} />
              <span className="truncate">{a.agent}</span>
            </span>
            <span className="text-right font-mono text-xs">{a.planCount}</span>
            <span className="text-right font-mono text-xs">
              {a.planCount > 0 ? formatTokens(a.tokens / a.planCount) : "—"}
            </span>
            <span className="text-right font-mono text-xs">{formatTokens(a.tokens)}</span>
            <span className="text-right font-mono text-xs">{formatCurrency(a.cost)}</span>
            <span className="flex items-center gap-2">
              <Bar value={share} className="flex-1" height={5} />
              <span className="w-9 text-right font-mono text-[11px] text-muted-foreground">{Math.round(share)}%</span>
            </span>
          </div>
        );
      })}
    </Card>
  );
};
