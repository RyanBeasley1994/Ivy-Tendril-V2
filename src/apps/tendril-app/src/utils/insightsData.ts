import { formatDate, type Formatters } from "@ivy-interactive/components/i18n";
import type {
  DashboardKpiDto,
  DashboardMonthValueDto,
  DashboardTrendDto,
} from "@ivy-interactive/components/tendril";
import type { TFunction } from "../i18n";
import type { DashboardActivity, RecentMergedPr } from "../types/api";
import { NO_VALUE } from "./dashboardMetrics";
import { rollingAverage, toDayNumber, toIsoDate, todayDayNumber } from "./rollingAverage";

/**
 * The analytics series the Insights page plots: the 28-day cost and plan trend, merged pull requests
 * per week, and the four KPI cards V1's dashboard carried. They moved here from `DashboardView` when
 * the dashboard became the command center, which shows what is live rather than what it cost.
 */

/** Days the trend card plots, from `DashboardApp.TrendDailyWindowDays`. */
const TREND_DAILY_WINDOW_DAYS = 28;

/** Weeks the Pull Requests card's Week tab plots, from `DashboardApp.BuildWeeklyPullRequests`. */
const PR_WEEKS_SHOWN = 6;

/**
 * The header's date line (`DashboardApp.Build`): weekday, day and month, in the current language's
 * own order - "Monday, September 15" in English, "Montag, 15. September" in German.
 *
 * V1 writes "Monday, 15th September", composed by hand with an English ordinal suffix
 * (`DashboardApp.Ordinal`). No `Intl` option produces English ordinals, and a hand-built
 * `weekday, ordinal month` cannot be reordered for any other language, so this is `Intl`'s form.
 */
export const formatDateText = (now: Date, format: Formatters): string =>
  format.date(now, { weekday: "long", day: "numeric", month: "long" });

/**
 * The trend card's series: four weeks of contiguous days ending today, with the 7-day trailing mean
 * beside them. This is `DashboardApp.BuildDailyTrend`, and it lives in the view for the same reason
 * V1 keeps it in `DashboardApp` rather than in the widget: the window is the page's decision.
 *
 * Zero-filled, so a day with no rows is a plotted 0 rather than a missing point; the *rolling*
 * series is the one that carries nulls, for the days its window would reach back past the earliest
 * record. Returns null only when there is no activity payload at all - V1's
 * `DailyCosts == null && DailyPlans == null`, which in C# means the daemon never supplied the
 * fields.
 *
 * An *empty* pair of arrays is not that. V1's models declare them nullable and its repository
 * always assigns a list, so null there means "absent"; V2's DTO types them `T[]`, and the daemon
 * always sends them, so the only value a fresh install ever produces is `[]`. Porting the null
 * check as `.length === 0` therefore turned "no payload" into "no rows yet" and withheld the card
 * from exactly the installs that have nothing else on the page - a 28-day axis of zeroes is the
 * honest answer there, and the same one V1 gives.
 *
 * Suppressing it also broke the layout: `.tdb-col:not(.tdb-col-side) > :last-child` stretches the
 * main column's final child so the trend card absorbs the leftover height. With the card gone that
 * fell to the KPI grid, which rendered 527px tall instead of 134px.
 */
export function buildDailyTrend(
  activity: DashboardActivity | null,
  today: number = todayDayNumber(),
): DashboardTrendDto | null {
  if (activity == null) return null;

  const costByDay = new Map(activity.dailyCosts.map((day) => [day.date, day.cost]));
  const plansByDay = new Map(activity.dailyPlans.map((day) => [day.date, day.count]));
  const dates = Array.from({ length: TREND_DAILY_WINDOW_DAYS }, (_, index) =>
    toIsoDate(today - TREND_DAILY_WINDOW_DAYS + 1 + index),
  );
  const costAt = (isoDate: string): number => costByDay.get(isoDate) ?? 0;
  const plansAt = (isoDate: string): number => plansByDay.get(isoDate) ?? 0;

  return {
    dates,
    cost: dates.map(costAt),
    plans: dates.map(plansAt),
    rollingCost: rollingAverage(dates, costAt, activity.dailyDataStart),
    rollingPlans: rollingAverage(dates, plansAt, activity.dailyDataStart),
  };
}

/**
 * Merged PRs per week for the Pull Requests card's Week tab, which is
 * `DashboardApp.BuildWeeklyPullRequests`: six Monday-to-Sunday weeks ending with the week containing
 * today, labelled by the week's start date.
 *
 * The tab exists in the widget and is clickable, so leaving `pullRequestsWeekly` unsupplied is not a
 * missing feature — it is a control that renders an empty chart. V1 feeds it from
 * `GetCompletedPrsByDay`; V2's daemon exposes no per-day PR series, so this buckets the merged-PR
 * rows themselves, which count the same thing (a `PullRequests` row on a Completed plan, dated by
 * the plan's `Updated`).
 *
 * The one caveat is the list's server-side `LIMIT`: it is the *most recent* rows, so the newest
 * weeks are always complete and only the oldest can be truncated. See the report note on raising
 * `useDashboardAnalytics`'s limit.
 */
export function buildWeeklyPullRequests(
  mergedPrs: readonly RecentMergedPr[],
  today: number = todayDayNumber(),
): DashboardMonthValueDto[] {
  // Epoch day 0 is a Thursday, whose distance from the preceding Monday is 3.
  const currentWeekMonday = today - ((today + 3) % 7);

  const countByDay = new Map<number, number>();
  for (const pr of mergedPrs) {
    const day = toDayNumber(pr.updated.slice(0, 10));
    if (day == null) continue;
    countByDay.set(day, (countByDay.get(day) ?? 0) + 1);
  }

  return Array.from({ length: PR_WEEKS_SHOWN }, (_unused, index) => {
    const weekStart = currentWeekMonday - (PR_WEEKS_SHOWN - 1 - index) * 7;
    let value = 0;
    for (let offset = 0; offset < 7; offset++) value += countByDay.get(weekStart + offset) ?? 0;

    const startDate = new Date(toIsoDate(weekStart) + "T00:00:00Z");
    return {
      // "Sep 14" in English: the week's start, in the current language's short month-and-day form.
      label: formatDate(startDate, { month: "short", day: "numeric", timeZone: "UTC" }),
      value,
      year: startDate.getUTCFullYear(),
      month: startDate.getUTCMonth() + 1,
      day: startDate.getUTCDate(),
      date: toIsoDate(weekStart),
    };
  });
}

/**
 * The KPI cards V1 shows, in V1's order (`DashboardApp.BuildKpis`). Its fourth card is the agent
 * rate-limit window, which falls back to Avg Cost/Plan when no usage snapshot exists — and V2 has
 * no usage service, so that branch is permanent. Anything the metrics helper computes beyond these
 * four is not a card V1 has.
 */
export const KPI_IDS = ["featuresShipped", "costPerFeature", "forecastMonth", "avgCostPlan"];

/**
 * The same four cards once the daemon has answered and had nothing to say, carrying V1's own no-data
 * vocabulary (`DashboardApp.BuildKpis`, `BuildForecastKpi`): an unknown figure is a dash or "n/a" with
 * a hint saying why, never a zero. None carries an `id`, so nothing is clickable while there is no
 * data behind the drill-down. An offline daemon must degrade the page, never blank it or change which
 * cards it has.
 *
 * This is the *settled* no-data state only. It used to double as the loading state, because the view
 * read `activity == null` — which is equally true of a fetch still in flight — and that conflation is
 * where the flash of dashes and "n/a" on every visit to the Dashboard came from. A figure nobody has
 * fetched yet is not an unknown figure; it gets a skeleton.
 *
 * Built per render rather than once, so its wording follows the language.
 */
export const fallbackKpis = (t: TFunction<"dashboard">): DashboardKpiDto[] => [
  {
    label: t("fallbackKpis.featuresShipped.label"),
    icon: "rocket",
    value: NO_VALUE,
    hint: t("fallbackKpis.featuresShipped.hint"),
  },
  {
    label: t("fallbackKpis.costPerFeature.label"),
    icon: "coins",
    value: t("fallbackKpis.costPerFeature.value"),
    hint: t("fallbackKpis.costPerFeature.hint"),
  },
  {
    label: t("fallbackKpis.forecastMonth.label"),
    icon: "calendar",
    value: NO_VALUE,
    hint: t("fallbackKpis.forecastMonth.hint"),
  },
  {
    label: t("fallbackKpis.avgCostPlan.label"),
    icon: "receipt",
    value: NO_VALUE,
    hint: t("fallbackKpis.avgCostPlan.hint"),
  },
];

