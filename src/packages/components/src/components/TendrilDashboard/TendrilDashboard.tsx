import React, { useState } from "react";
import {
  ArrowUp,
  CalendarClock,
  Check,
  CircleAlert,
  Clock,
  Coins,
  Eye,
  Feather,
  Info,
  Layers,
  LoaderCircle,
  type LucideIcon,
  MessageSquareWarning,
  Plus,
  Receipt,
  Rocket,
  Sprout,
  TrendingDown,
  TrendingUp,
  XCircle,
} from "lucide-react";
import {
  type DashboardAttentionDto,
  type DashboardKpiDto,
  type TendrilDashboardProps,
  formatCountTick,
  formatCurrencyTick,
  hasSlotContent,
} from "./types.ts";
import { TrendChart } from "./TrendChart.tsx";
import { ActivityGrid } from "./ActivityGrid.tsx";
import { PillBars } from "./PillBars.tsx";
import { Sparkline } from "./Sparkline.tsx";
import { TokenHeatmap } from "./TokenHeatmap.tsx";
import { PlanFlow } from "./PlanFlow.tsx";
import { ChartSkeleton, KpiSkeletonGrid } from "./DashboardSkeleton.tsx";
import { TuiBadge } from "../ui/TuiBadge";
import { formatCurrency, useTranslation } from "@/i18n/uiShell";
import "../ui/ui.css";
import "./dashboard.css";

interface StatusItemProps {
  icon: React.ReactNode;
  count: number;
  label: string;
  tone: string;
  onClick: () => void;
}

const StatusItem: React.FC<StatusItemProps> = ({ icon, count, label, tone, onClick }) => (
  <button type="button" className="tdb-status-item" data-tone={tone} onClick={onClick}>
    {icon}
    <span className="tdb-status-count">{count}</span>
    <span className="tdb-status-label">{label}</span>
  </button>
);

const KPI_ICONS: Record<string, LucideIcon> = {
  rocket: Rocket,
  coins: Coins,
  calendar: CalendarClock,
  receipt: Receipt,
  layers: Layers,
  eye: Eye,
};

const ATTENTION_ICONS: Record<DashboardAttentionDto["kind"], LucideIcon> = {
  review: Eye,
  failed: XCircle,
  stalled: Clock,
  info: Info,
};

const WHOLE_DOLLARS: Intl.NumberFormatOptions = {
  minimumFractionDigits: 0,
  maximumFractionDigits: 0,
};

/** Ungrouped like the old `toFixed(2)`: $999.996 rounds to "$1000.00", not "$1,000.00". */
const CENTS: Intl.NumberFormatOptions = {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
  useGrouping: false,
};

/**
 * A trend value: whole dollars from $1,000 up, cents below, in the current language. Rounded by
 * `Math.round` and `toFixed` first, as before: `Intl` rounds ties in a number's shortest decimal form
 * ($87.455 to $87.46) where `toFixed` rounds its binary value ($87.45).
 */
const formatCurrencyValue = (value: number): string =>
  value >= 1000
    ? formatCurrency(Math.round(value), "USD", WHOLE_DOLLARS)
    : formatCurrency(Number(value.toFixed(2)), "USD", CENTS);

const KpiBody: React.FC<{ kpi: DashboardKpiDto }> = ({ kpi }) => {
  const Icon = kpi.icon ? KPI_ICONS[kpi.icon] : undefined;
  return (
    <>
      <div className="tdb-kpi-head">
        {Icon && (
          <span className="tdb-kpi-icon" aria-hidden="true">
            <Icon size={16} />
          </span>
        )}
        <div className="tdb-kpi-label">{kpi.label}</div>
      </div>
      <div className="tdb-kpi-foot">
        <div className="tdb-kpi-figures">
          <div className="tdb-kpi-row">
            <span className="tdb-kpi-value">{kpi.value}</span>
            {kpi.subValue && <span className="tdb-kpi-subvalue">{kpi.subValue}</span>}
          </div>
          {kpi.delta && (
            <span className="tdb-kpi-delta" data-direction={kpi.direction ?? "up"}>
              {kpi.direction === "down" ? <TrendingDown /> : <TrendingUp />}
              {kpi.delta}
            </span>
          )}
        </div>
        {kpi.series && kpi.series.length > 1 && (
          <Sparkline className="tdb-kpi-spark" values={kpi.series} />
        )}
      </div>
      {kpi.hint && <div className="tdb-kpi-hint">{kpi.hint}</div>}
    </>
  );
};

export const TendrilDashboard: React.FC<TendrilDashboardProps> = ({
  id,
  events = [],
  eventHandler,
  dateText = "",
  greeting = "",
  headline = "",
  draftCount = 0,
  inProgressCount = 0,
  reviewCount = 0,
  completedCount = 0,
  failedCount = 0,
  kpis = [],
  trend = null,
  trendWeekly = null,
  pullRequests = [],
  pullRequestsWeekly = [],
  activity = [],
  jobs = [],
  statusText,
  statusOk = true,
  flow,
  flowFailed,
  attention,
  tokensDaily,
  tokenShare,
  loading = false,
  slots,
}) => {
  const { t } = useTranslation("uiShell");
  const [tab, setTab] = useState<"cost" | "plans">("cost");
  const [prPeriod, setPrPeriod] = useState<"week" | "month">("week");
  const [draft, setDraft] = useState("");
  const activePrs = prPeriod === "week" ? (pullRequestsWeekly ?? []) : (pullRequests ?? []);

  const fireEvent = (eventName: string, args: unknown[] = []) => {
    if (events.includes(eventName)) {
      eventHandler(eventName, id, args);
    }
  };

  const fireJobEvent = (jobId: string) => fireEvent("OnJob", [jobId]);
  const fireKpiEvent = (kpiId: string) => fireEvent("OnSelectKpi", [kpiId]);

  const composerEnabled = events.includes("OnCompose");
  const submitDraft = (e: React.FormEvent) => {
    e.preventDefault();
    fireEvent("OnCompose", [draft.trim()]);
    setDraft("");
  };

  const statusItems = [
    {
      id: "plans",
      icon: <Feather size={16} />,
      count: draftCount,
      label: t("dashboard.status.plans"),
      event: "OnDrafts",
      tone: "draft",
    },
    {
      id: "inProgress",
      icon: <Sprout size={16} />,
      count: inProgressCount,
      label: t("dashboard.status.inProgress"),
      event: "OnJobs",
      tone: "active",
    },
    {
      id: "review",
      icon: <Eye size={16} />,
      count: reviewCount,
      label: t("dashboard.status.readyForReview"),
      event: "OnReview",
      tone: "review",
    },
    {
      id: "completed",
      icon: <Check size={16} />,
      count: completedCount,
      label: t("dashboard.status.completed"),
      event: "OnJobs",
      tone: "done",
    },
    {
      id: "failed",
      icon: <MessageSquareWarning size={16} />,
      count: failedCount,
      label: t("dashboard.status.failed"),
      event: "OnJobs",
      tone: "failed",
    },
  ];

  const trendName = t("dashboard.trend.currentName");
  const formatPlansValue = (value: number): string =>
    t("dashboard.trend.plansValue", { count: Math.round(value) });

  const activeTrend = trendWeekly ?? trend;

  const trendData =
    activeTrend == null
      ? null
      : tab === "cost"
        ? {
            values: activeTrend.cost,
            rolling: activeTrend.rollingCost,
            formatTick: formatCurrencyTick,
            formatValue: formatCurrencyValue,
          }
        : {
            values: activeTrend.plans,
            rolling: activeTrend.rollingPlans,
            formatTick: formatCountTick,
            formatValue: formatPlansValue,
          };

  const hasJobDetail = jobs.some((job) => job.phase || job.tokens || job.elapsed);

  const composerEl = (
    <>
      {composerEnabled && (
        <form className="tdb-composer" onSubmit={submitDraft}>
          <Sprout size={16} className="tdb-composer-icon" aria-hidden="true" />
          <input
            className="tdb-composer-input"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={t("dashboard.composer.placeholder")}
            aria-label={t("dashboard.composer.ariaLabel")}
          />
          <button
            type="submit"
            className="tdb-composer-submit"
            aria-label={t("dashboard.composer.submit")}
            title={t("dashboard.composer.submit")}
          >
            <ArrowUp size={16} />
          </button>
        </form>
      )}
    </>
  );
  const kpisEl = (
    <>
      {/* Placeholders while no figures exist yet, so the row is never four dashes. Once they
                have arrived a refresh keeps rendering them: `loading` goes false and stays false. */}
      {loading && <KpiSkeletonGrid />}

      {!loading && kpis.length > 0 && (
        <div className="tdb-kpis">
          {kpis.map((kpi, index) => {
            const kpiId = kpi.id;
            const key = kpiId ?? kpi.label;

            return kpiId == null ? (
              <div className="tdb-kpi" data-tone={index % 4} key={key}>
                <KpiBody kpi={kpi} />
              </div>
            ) : (
              <button
                type="button"
                className="tdb-kpi"
                data-clickable="true"
                data-tone={index % 4}
                key={key}
                aria-label={t("dashboard.kpi.breakdownAriaLabel", { label: kpi.label })}
                onClick={() => fireKpiEvent(kpiId)}
              >
                <KpiBody kpi={kpi} />
              </button>
            );
          })}
        </div>
      )}
    </>
  );
  const attentionEl = (
    <>
      {attention && (
        <div className="tdb-block tdb-attention">
          <div className="tdb-side-head">
            <div className="tdb-block-title">{t("dashboard.attention.title")}</div>
            {attention.length > 0 && (
              <span className="tdb-count-pill">
                {t("dashboard.attention.count", { count: attention.length })}
              </span>
            )}
          </div>
          {attention.length === 0 ? (
            <div className="tdb-attention-empty">
              <Check size={16} aria-hidden="true" />
              {t("dashboard.attention.empty")}
            </div>
          ) : (
            <ul className="tdb-attention-list">
              {attention.map((item) => {
                const Icon = ATTENTION_ICONS[item.kind] ?? CircleAlert;
                return (
                  <li key={item.id} className="tdb-attention-row" data-kind={item.kind}>
                    <span className="tdb-attention-icon" aria-hidden="true">
                      <Icon size={15} />
                    </span>
                    <div className="tdb-attention-text">
                      <div className="tdb-attention-title">
                        {item.title}
                        {item.status && <span className="tdb-attention-status">{item.status}</span>}
                      </div>
                      {item.detail && <div className="tdb-attention-detail">{item.detail}</div>}
                    </div>
                    <button
                      type="button"
                      className="tdb-attention-action"
                      data-primary={item.kind === "review"}
                      onClick={() => fireEvent("OnAttention", [item.id])}
                    >
                      {item.action}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}
    </>
  );
  const jobsEl = (
    <>
      {/* Live jobs and the token heatmap share the grid's second row, so their tops and
              bottoms line up across the two columns. */}
      <div className="tdb-block tdb-jobs">
        <div className="tdb-side-head">
          <div className="tdb-block-title">{t("dashboard.jobs.title")}</div>
          {jobs.length > 0 && <span className="tdb-count-pill">{jobs.length}</span>}
        </div>
        {hasJobDetail && jobs.length > 0 && (
          <div className="tdb-job-head" aria-hidden="true">
            <span />
            <span>{t("dashboard.jobs.columns.plan")}</span>
            <span>{t("dashboard.jobs.columns.phase")}</span>
            <span className="tdb-num">{t("dashboard.jobs.columns.tokens")}</span>
            <span className="tdb-num">{t("dashboard.jobs.columns.time")}</span>
          </div>
        )}
        <div className="tdb-jobs-list hidden-scrollbar">
          {jobs.length === 0 && <div className="tdb-empty-note">{t("dashboard.jobs.empty")}</div>}
          {jobs.map((job) => (
            <button
              key={job.id}
              type="button"
              className="tdb-job-row"
              data-detailed={hasJobDetail}
              data-stalled={job.stalled === true}
              onClick={() => fireJobEvent(job.id)}
            >
              <LoaderCircle
                size={14}
                className="tdb-job-spinner"
                data-spinning={job.status === "running" && !job.stalled}
              />
              <span className="tdb-job-main">
                {job.planId && (
                  <TuiBadge className="tdb-job-tag" size="md" numeric>
                    {job.planId}
                  </TuiBadge>
                )}
                <span className="tdb-job-title">{job.title}</span>
              </span>
              {hasJobDetail && (
                <>
                  <span className="tdb-job-phase">
                    {job.stalled ? t("dashboard.jobs.stalled") : job.phase}
                  </span>
                  <span className="tdb-num tdb-job-tokens">{job.tokens ?? "—"}</span>
                  <span className="tdb-num tdb-job-elapsed">{job.elapsed ?? "—"}</span>
                </>
              )}
            </button>
          ))}
        </div>
      </div>
    </>
  );
  const tokensEl = (
    <>
      {tokensDaily != null ? (
        <div className="tdb-block tdb-tokens">
          <div className="tdb-block-title">{t("dashboard.tokens.title")}</div>
          {loading && tokensDaily.length === 0 ? (
            <ChartSkeleton label={t("dashboard.tokens.loading")} />
          ) : (
            <TokenHeatmap days={tokensDaily} share={tokenShare} />
          )}
        </div>
      ) : (
        <div className="tdb-grid-filler" aria-hidden="true" />
      )}
    </>
  );

  // The command-center layout, which is what the app renders: header and actions, the composer, four
  // KPI tiles, then Plan Flow beside Needs Attention and Live Jobs beside Tokens. A host that passes
  // no `flow` keeps the original layout below, unchanged.
  if (flow) {
    return (
      <div className="tdb-root" data-layout="command">
        <div className="tdb-inner tcc">
          <header className="tcc-head">
            <div className="tcc-head-text">
              <h1 className="tcc-greeting">{greeting}</h1>
              {statusText && (
                <div className="tdb-status-text" data-ok={statusOk}>
                  <span className="tdb-live-dot" aria-hidden="true" />
                  {statusText}
                </div>
              )}
            </div>
            {events.includes("OnNewPlan") && (
              <button type="button" className="tcc-primary" onClick={() => fireEvent("OnNewPlan")}>
                <Plus size={14} strokeWidth={2.4} aria-hidden="true" />
                {t("dashboard.newPlan")}
              </button>
            )}
          </header>

          {composerEl}
          {kpisEl}

          <div className="tcc-row">
            <section className="tdb-block tcc-flow-card">
              <div className="tdb-side-head">
                <div className="tdb-block-title">{t("dashboard.flow.title")}</div>
                <span className="tcc-live-pill">
                  <span className="tdb-live-dot" aria-hidden="true" />
                  {t("dashboard.flow.live")}
                </span>
                <span className="tcc-caption">{t("dashboard.flow.caption")}</span>
              </div>
              <PlanFlow
                stages={flow}
                failed={flowFailed}
                onSelect={(stageId) => fireEvent("OnFlowStage", [stageId])}
              />
            </section>
            {attentionEl}
          </div>

          <div className="tcc-row tcc-row-fill">
            {jobsEl}
            {tokensEl}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="tdb-root">
      <div className="tdb-inner">
        <div className="tdb-grid">
          <div className="tdb-col">
            <header className="tdb-header">
              {dateText && <div className="tdb-date">{dateText}</div>}
              <h1 className="tdb-greeting">{greeting}</h1>
              <h1 className="tdb-headline">{headline}</h1>
              {statusText && (
                <div className="tdb-status-text">
                  <span className="tdb-live-dot" aria-hidden="true" />
                  {statusText}
                </div>
              )}
            </header>

            {composerEl}

            <div className="tdb-block tdb-status">
              {statusItems.map((item, index) => (
                <React.Fragment key={item.id}>
                  {index > 0 && <div className="tdb-status-sep" />}
                  <StatusItem
                    icon={item.icon}
                    count={item.count}
                    label={item.label}
                    tone={item.tone}
                    onClick={() => fireEvent(item.event)}
                  />
                </React.Fragment>
              ))}
            </div>

            {kpisEl}

            <div className="tdb-block tdb-factory">
              <div className="tdb-block-title">{t("dashboard.factory.title")}</div>
              <div className="tdb-factory-body">{slots?.ProcessViewer}</div>
            </div>

            {/* The card itself, not just its chart: drawing the frame while loading is what stops the
                column reflowing when a series arrives. Tabs and legend wait for a series. */}
            {loading && (
              <div className="tdb-block tdb-trend">
                <div className="tdb-trend-chart">
                  <ChartSkeleton label={t("dashboard.trend.loading")} />
                </div>
              </div>
            )}

            {!loading && trendData && (
              <div className="tdb-block tdb-trend">
                <div className="tdb-trend-header">
                  <div className="tdb-tabs">
                    <button
                      type="button"
                      className="tdb-tab"
                      data-active={tab === "cost"}
                      onClick={() => setTab("cost")}
                    >
                      {t("dashboard.trend.costTab")}
                    </button>
                    <button
                      type="button"
                      className="tdb-tab"
                      data-active={tab === "plans"}
                      onClick={() => setTab("plans")}
                    >
                      {t("dashboard.trend.plansTab")}
                    </button>
                  </div>
                  <div className="tdb-trend-sep" />
                  <div className="tdb-legend">
                    <span className="tdb-legend-item">
                      <span className="tdb-legend-dot" />
                      {trendName}
                    </span>
                    <span className="tdb-legend-item">
                      <span className="tdb-legend-line-avg" />
                      {t("dashboard.trend.rollingLegend")}
                    </span>
                  </div>
                </div>
                <div className="tdb-trend-chart">
                  <TrendChart
                    dates={activeTrend!.dates}
                    values={trendData.values}
                    rolling={trendData.rolling}
                    currentName={trendName}
                    formatTick={trendData.formatTick}
                    formatValue={trendData.formatValue}
                  />
                </div>
              </div>
            )}
          </div>

          <div className="tdb-col tdb-col-side">
            <div className="tdb-update-slot">{slots?.UpdateNotice}</div>

            {attentionEl}

            <div className="tdb-block tdb-side-block">
              <div className="tdb-block-title">{t("dashboard.gitActivity.title")}</div>
              <div className="tdb-side-body">
                {loading ? (
                  <ChartSkeleton label={t("dashboard.gitActivity.loading")} />
                ) : (
                  <ActivityGrid months={activity} />
                )}
              </div>
            </div>
            <div className="tdb-block tdb-side-block">
              <div className="tdb-side-head">
                <div className="tdb-block-title">{t("dashboard.pullRequests.title")}</div>
                <div className="tdb-tabs tdb-side-tabs">
                  <button
                    type="button"
                    className="tdb-tab tdb-side-tab"
                    data-active={prPeriod === "week"}
                    onClick={() => setPrPeriod("week")}
                  >
                    {t("dashboard.pullRequests.weekTab")}
                  </button>
                  <button
                    type="button"
                    className="tdb-tab tdb-side-tab"
                    data-active={prPeriod === "month"}
                    onClick={() => setPrPeriod("month")}
                  >
                    {t("dashboard.pullRequests.monthTab")}
                  </button>
                </div>
              </div>
              <div className="tdb-side-body">
                {loading ? (
                  <ChartSkeleton label={t("dashboard.pullRequests.loading")} />
                ) : (
                  <PillBars items={activePrs} />
                )}
              </div>
            </div>
            {hasSlotContent(slots?.TunnelQr) && (
              <div className="tdb-block tdb-side-block tdb-tunnel">
                <div className="tdb-side-head">
                  <div className="tdb-block-title">{t("dashboard.tunnel.title")}</div>
                  {slots?.TunnelMenu}
                </div>
                <div className="tdb-tunnel-body">{slots?.TunnelQr}</div>
              </div>
            )}
          </div>

          {jobsEl}

          {tokensEl}
        </div>
      </div>
    </div>
  );
};
