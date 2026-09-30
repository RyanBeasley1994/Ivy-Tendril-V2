import type React from "react";
import { formatCompact, formatCurrency, formatDate, formatNumber, i18n } from "@/i18n/uiShell";

export type IvyEventHandler = (eventName: string, widgetId: string, args: unknown[]) => void;

export interface DashboardKpiDto {
  label: string;
  value: string;
  delta?: string | null;
  direction?: "up" | "down" | null;
  /**
   * Stable identifier passed as the sole argument to `OnSelectKpi`. A KPI without one is not
   * clickable, which is what keeps a purely informational tile from opening an empty drill-down.
   */
  id?: string | null;
  /**
   * A second line under the value, for a figure that is not actionable without its basis (the cost
   * forecast states the day counts it projected from here). Null on the cards that need none.
   */
  hint?: string | null;
  /** A smaller figure beside the value, on a card whose number needs a second one to read. */
  subValue?: string | null;
  /** The recent values behind the figure, oldest first, drawn as a sparkline beside it. */
  series?: number[] | null;
  /** A glyph for the card's icon tile: one of {@link KPI_ICON_NAMES}. */
  icon?: string | null;
}

/** The glyphs a KPI tile can carry, by name, so a host can pick one without importing lucide. */
export const KPI_ICON_NAMES = ["rocket", "coins", "calendar", "receipt", "layers", "eye"] as const;

/**
 * One thing on the dashboard that is waiting on the operator: a plan ready for review, a failed
 * verification, a job that stopped producing output. `OnAttention` fires with the id.
 */
export interface DashboardAttentionDto {
  id: string;
  /** Drives the icon and colour: review is an invitation, failed and stalled are problems. */
  kind: "review" | "failed" | "stalled" | "info";
  title: string;
  /** A short status beside the title, e.g. "6/6 checks" or "no output 9m". */
  status?: string | null;
  /** The mono line underneath: plan id, name, age. */
  detail?: string | null;
  /** The button's label. */
  action: string;
}

/** One column of the Plan Flow ribbon. `OnFlowStage` fires with the id when it is clicked. */
export interface DashboardFlowStageDto {
  id: string;
  label: string;
  count: number;
}

/** Tokens used on one calendar day. */
export interface DashboardTokenDayDto {
  /** `yyyy-MM-dd`. */
  date: string;
  tokens: number;
}

/** One slice of the token split bar. */
export interface DashboardTokenShareDto {
  label: string;
  tokens: number;
}

export interface DashboardMonthValueDto {
  label: string;
  value: number;
  /**
   * The bucket's calendar position. Optional: a caller that supplies none gets `label` verbatim in
   * the tooltip and the accessible name, which is what the samples app relies on.
   */
  year?: number;
  month?: number;
  day?: number;
  date?: string;
}

export interface DashboardActivityDayDto {
  date: string;
  count: number;
}

export interface DashboardActivityMonthDto {
  label: string;
  weeks: number[];
  /** The month's daily breakdown, which is the only thing a week cell's date can be recovered from. */
  days?: DashboardActivityDayDto[];
}

export interface DashboardJobDto {
  id: string;
  planId: string;
  title: string;
  /** Lowercased job status; "running" animates the row's spinner. */
  status: string;
  /** What the job is doing, e.g. the promptware or the status message's first word. */
  phase?: string | null;
  /** Pre-formatted, e.g. "412k". */
  tokens?: string | null;
  /** Pre-formatted elapsed time, e.g. "08:42". */
  elapsed?: string | null;
  /** True when the job has stopped producing output; the row is tinted as a warning. */
  stalled?: boolean;
}

export interface DashboardTrendDto {
  /** One `yyyy-MM-dd` per plotted day, ascending and contiguous. Labels are derived from these. */
  dates: string[];
  cost: number[];
  plans: number[];
  /** 7-day trailing mean aligned to `dates`; null where the window reaches past the earliest record. */
  rollingCost: (number | null)[];
  rollingPlans: (number | null)[];
}

export interface TendrilDashboardProps {
  id: string;
  width?: string;
  height?: string;
  events?: string[];
  eventHandler: IvyEventHandler;
  dateText?: string;
  greeting?: string;
  headline?: string;
  draftCount?: number;
  inProgressCount?: number;
  reviewCount?: number;
  completedCount?: number;
  failedCount?: number;
  kpis?: DashboardKpiDto[];
  trend?: DashboardTrendDto | null;
  /** The short window, preferred over `trend` when supplied. */
  trendWeekly?: DashboardTrendDto | null;
  pullRequests?: DashboardMonthValueDto[];
  pullRequestsWeekly?: DashboardMonthValueDto[];
  activity?: DashboardActivityMonthDto[];
  jobs?: DashboardJobDto[];
  /** A one-line summary under the greeting, e.g. "5 agents running · 2 queued". */
  statusText?: string;
  /** False when the service is unreachable: the status dot turns red. */
  statusOk?: boolean;
  /** The plan lifecycle, in order. Omitted, the Plan Flow card is not drawn. */
  flow?: DashboardFlowStageDto[];
  /** Plans that failed out of the flow, drawn as a branch. */
  flowFailed?: DashboardFlowStageDto | null;
  /** What is waiting on the operator. Omitted, the Needs Attention card is not drawn. */
  attention?: DashboardAttentionDto[];
  /** Tokens per day, ascending. Omitted, the Tokens card is not drawn. */
  tokensDaily?: DashboardTokenDayDto[] | null;
  /** The split bar under the token heatmap, largest first. */
  tokenShare?: DashboardTokenShareDto[];
  /**
   * True while the analytics behind the KPI, trend, Git Activity and Pull Requests cards have never
   * arrived. Those four regions render `Skeleton` placeholders of their own shape instead, which is
   * the framework's answer to a pending region (`Ivy/Widgets/Primitives/Skeleton.cs`).
   *
   * It means "no figure exists yet", not "a request is in flight": a *refresh* must leave the last
   * numbers on screen, so a caller that still holds a previous snapshot passes false throughout. That
   * distinction is the whole point — the alternative is a tile that drops to a dash and back on every
   * poll, which is the flicker this prop exists to remove.
   *
   * The status strip and Active Jobs take no placeholder: their counts come from the plan and job
   * stores, which keep the previous list across a refresh and so never have nothing to show.
   */
  loading?: boolean;
  slots?: {
    ProcessViewer?: React.ReactNode;
    UpdateNotice?: React.ReactNode;
    TunnelQr?: React.ReactNode;
    TunnelMenu?: React.ReactNode;
  };
}

/** Slots arrive as arrays of rendered nodes; an omitted slot is undefined. */
export const hasSlotContent = (slot?: React.ReactNode): boolean =>
  slot != null && (!Array.isArray(slot) || slot.length > 0);

/** Grey ramp step (1-4) for an intensity relative to the range maximum. */
export const rampLevel = (value: number, max: number): number => {
  if (value <= 0 || max <= 0) return 0;
  const ratio = value / max;
  if (ratio <= 0.25) return 1;
  if (ratio <= 0.5) return 2;
  if (ratio <= 0.8) return 3;
  return 4;
};

/** "Nice" rounded axis ticks from 0 up to at least max, e.g. [0, 10, 20, 30]. */
export const niceTicks = (max: number, count = 3): number[] => {
  if (max <= 0) return [0, 1];
  const rawStep = max / count;
  const magnitude = Math.pow(10, Math.floor(Math.log10(rawStep)));
  const residual = rawStep / magnitude;
  const step = (residual > 5 ? 10 : residual > 2 ? 5 : residual > 1 ? 2 : 1) * magnitude;
  const ticks: number[] = [];
  for (let v = 0; v < max + step; v += step) ticks.push(Math.round(v * 1000) / 1000);
  return ticks;
};

const WHOLE_UNITS: Intl.NumberFormatOptions = {
  minimumFractionDigits: 0,
  maximumFractionDigits: 0,
};

/**
 * Compact digits for a tick of 1,000 or more: whole units below a million ("$30K"), one decimal from
 * a million up, so neighbouring ticks such as 1.5M and 2M do not both read "2M".
 */
const compactTickDigits = (value: number): Intl.NumberFormatOptions =>
  value >= 1_000_000 ? { minimumFractionDigits: 0, maximumFractionDigits: 1 } : WHOLE_UNITS;

/*
 * Axis ticks, in the current language at each call: "$30K", "$40", "2K", "150" in English, the
 * language's own currency placement and thousands abbreviation elsewhere. Whole units only below a
 * million, rounded half up as `Math.round` did; a zero tick stays a bare "0".
 */
export const formatCurrencyTick = (value: number): string => {
  if (value >= 1000) {
    return formatCurrency(value, "USD", { notation: "compact", ...compactTickDigits(value) });
  }
  return value === 0
    ? formatNumber(0)
    : formatCurrency(value, "USD", { ...WHOLE_UNITS, useGrouping: false });
};

export const formatCountTick = (value: number): string => {
  if (value >= 1000) return formatCompact(value, compactTickDigits(value));
  return formatNumber(value, { ...WHOLE_UNITS, useGrouping: false });
};

/** Arithmetic mean of non-empty number arrays, returning null when empty. */
export const computeAverage = (values: number[]): number | null => {
  if (!values || values.length === 0) return null;
  const sum = values.reduce((acc, val) => acc + val, 0);
  return sum / values.length;
};

export const ROLLING_WINDOW_DAYS = 7;

/**
 * Trailing mean of each entry and up to `window - 1` before it, calculating an expanding
 * average for leading entries with fewer days than `window`. Only a fallback: the host sends
 * a rolling series that also sees the days before the displayed range, and this one cannot.
 */
export const computeRollingAverage = (
  values: number[],
  window = ROLLING_WINDOW_DAYS,
): (number | null)[] =>
  values.map((_, index) =>
    index < window - 1
      ? values.slice(0, index + 1).reduce((acc, v) => acc + v, 0) / (index + 1)
      : values.slice(index - window + 1, index + 1).reduce((acc, value) => acc + value, 0) / window,
  );

/**
 * Splits `yyyy-MM-dd` by hand rather than through `new Date(iso)`: the string form parses as UTC
 * midnight and reports the previous day in every negative-offset time zone.
 */
const parseIsoDate = (iso: string): { year: number; month: number; day: number } | null => {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!parts) return null;
  const month = Number(parts[2]);
  if (month < 1 || month > 12) return null;
  return { year: Number(parts[1]), month, day: Number(parts[3]) };
};

/** A parsed day as a local-midnight `Date`, which is what `Intl` formats back to the same day. */
const localDate = ({ year, month, day }: { year: number; month: number; day: number }): Date =>
  new Date(year, month - 1, day);

/**
 * Axis tick for a day, in the current language at each call: "Sep 7", or "Sep 7 '25" when the year
 * has to be spelled out.
 */
export const formatAxisDate = (iso: string, includeYear = false): string => {
  const parsed = parseIsoDate(iso);
  if (!parsed) return iso;
  const base = formatDate(localDate(parsed), { month: "short", day: "numeric" });
  return includeYear
    ? i18n.t("uiShell:trendChart.axisDateWithYear", {
        date: base,
        year: String(parsed.year).slice(-2),
      })
    : base;
};

/** Tooltip title for a day, in the current language at each call: "Sun, Sep 7, 2026". */
export const formatTooltipDate = (iso: string): string => {
  const parsed = parseIsoDate(iso);
  if (!parsed) return iso;
  return formatDate(localDate(parsed), {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: "numeric",
  });
};
