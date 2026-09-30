import React from "react";
import { useTranslation } from "../i18n";
import { useDashboardAnalytics } from "../hooks/useDashboardAnalytics";
import { NO_VALUE, buildKpis } from "../utils/dashboardMetrics";
import { KPI_IDS, fallbackKpis } from "../utils/insightsData";
import { formatTokens } from "../utils/commandCenter";
import { lastDays } from "../utils/fleet";
import { toIsoDate, todayDayNumber } from "../utils/rollingAverage";
import { DashboardKpiSheet } from "./sheets/DashboardKpiSheet";
import { HeatmapCard } from "./command/panels";
import {
  AgentsCard,
  CostliestPlansCard,
  DotMatrixCard,
  KpiStrip,
  MonthSpendCard,
  type InsightKpi,
} from "./insights/panels";

const TOKEN_WINDOW_DAYS = 30;

/** Series behind each KPI, for its mini bars: the last 12 days of whatever it counts. */
const KPI_COLORS: Record<string, string> = {
  featuresShipped: "#19e0a5",
  costPerFeature: "var(--info)",
  forecastMonth: "var(--violet)",
  avgCostPlan: "var(--info)",
};

/**
 * The analytics page: what Tendril has cost and produced, by day, by agent and by plan. The command
 * center is the live view; this is the ledger. Every KPI opens the same drill-down sheet as before.
 */
export const InsightsView: React.FC<{ onSelectPlan?: (planId: string) => void }> = ({ onSelectPlan }) => {
  const { t } = useTranslation("dashboard");
  const analytics = useDashboardAnalytics();
  const { activity, agentCosts } = analytics;
  const [selectedKpi, setSelectedKpi] = React.useState<string | null>(null);
  const analyticsPending = analytics.loading && activity == null;

  const today = todayDayNumber();
  const tokens12 = lastDays(activity, "tokens", 12);
  const cost12 = lastDays(activity, "cost", 12);
  const tokensByDay = new Map((activity?.dailyCosts ?? []).map((d) => [d.date, d.tokens]));
  const tokens30 = Array.from({ length: TOKEN_WINDOW_DAYS }, (_, i) =>
    tokensByDay.get(toIsoDate(today - TOKEN_WINDOW_DAYS + 1 + i)) ?? 0,
  ).reduce((sum, v) => sum + v, 0);
  const shipped = new Map(analytics.shippedFeatures.map((d) => [d.date, d.count]));
  const shipped12 = Array.from({ length: 12 }, (_, i) => shipped.get(toIsoDate(today - 11 + i)) ?? 0);

  const kpis =
    activity != null
      ? buildKpis({
          activity,
          shippedFeatures: analytics.shippedFeatures,
          planCosts: analytics.planCosts,
        }).filter((kpi) => kpi.id != null && KPI_IDS.includes(kpi.id))
      : fallbackKpis(t);

  const seriesFor = (id: string | null | undefined): number[] | undefined =>
    id === "featuresShipped" ? shipped12 : id === "forecastMonth" ? cost12 : undefined;

  const items: InsightKpi[] = [
    {
      id: "tokens",
      label: t("insights.tokens30"),
      value: activity ? (formatTokens(tokens30) ?? NO_VALUE) : NO_VALUE,
      series: tokens12,
      color: "#19e0a5",
    },
    ...kpis.map(
      (kpi): InsightKpi => ({
        id: kpi.id ?? kpi.label,
        label: kpi.label,
        value: kpi.value,
        note: kpi.delta ?? null,
        tone: kpi.delta ? (kpi.direction === "down" ? "info" : "ok") : "mute",
        hint: kpi.hint ?? null,
        series: seriesFor(kpi.id),
        color: kpi.id ? KPI_COLORS[kpi.id] : undefined,
        onSelect: kpi.id ? () => setSelectedKpi(kpi.id!) : undefined,
      }),
    ),
  ];

  return (
    <div data-testid="insights-view" className="flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto px-6 pb-6 pt-5">
      <header className="flex shrink-0 items-end gap-2.5">
        <div className="flex flex-col gap-1.5">
          <h1 className="m-0 text-[26px] font-semibold tracking-[-0.025em]">{t("insights.title")}</h1>
          <p className="m-0 text-[13px] text-muted-foreground">{t("insights.subtitle")}</p>
        </div>
      </header>

      <KpiStrip items={items} loading={analyticsPending} />

      <div className="grid shrink-0 grid-cols-1 gap-3.5 xl:grid-cols-12">
        <DotMatrixCard className="xl:col-span-8" activity={activity} />
        <MonthSpendCard
          className="xl:col-span-4"
          activity={activity}
          agents={agentCosts}
          onOpenForecast={activity ? () => setSelectedKpi("forecastMonth") : undefined}
        />
      </div>

      <div className="grid shrink-0 grid-cols-1 gap-3.5 xl:grid-cols-12">
        <HeatmapCard className="xl:col-span-7" activity={activity} weeks={52} cell={9} title={t("insights.v2.year")} />
        <CostliestPlansCard
          className="xl:col-span-5"
          plans={analytics.planCosts}
          onOpenPlan={(id) => onSelectPlan?.(id)}
        />
      </div>

      <AgentsCard agents={agentCosts} className="shrink-0" />

      <DashboardKpiSheet kpiId={selectedKpi} analytics={analytics} onClose={() => setSelectedKpi(null)} />
    </div>
  );
};
