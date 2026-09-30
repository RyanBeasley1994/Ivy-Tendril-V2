import type { DashboardActivity, Job, PlanSummary, RecentMergedPr } from "../types/api";
import { ACTIVE_JOB_STATUSES } from "./processStatus";
import { stalledMinutes } from "./commandCenter";
import { toIsoDate, todayDayNumber } from "./rollingAverage";

/**
 * The V2 command center's derived data: the agent-fleet timeline, the decisions queue, the activity
 * feed and the heatmap's summary figures. Pure functions over the stores' plans and jobs and the
 * analytics payload, so the view does no arithmetic of its own.
 */

export const FLEET_WINDOW_MINUTES = 60;
export const FLEET_LANES = 6;

const MINUTE_MS = 60_000;
const DAY_MS = 86_400_000;

/** What a timeline segment is drawn as. */
export type SegmentKind = "explore" | "implement" | "ship" | "stalled" | "failed" | "done";

export interface FleetSegment {
  kind: SegmentKind;
  /** Start and end as a percentage of the window, 0 to 100. */
  from: number;
  to: number;
}

export interface FleetLane {
  job: Job;
  segments: FleetSegment[];
  /** Short state for the lane's pill. */
  state: "running" | "stalled" | "queued" | "failed" | "done";
  stalledFor: number | null;
}

const EXPLORE_TYPES = ["CreatePlan", "UpdatePlan", "ExpandPlan", "SplitPlan", "CreateIssue"];
const SHIP_TYPES = ["CreatePr", "SyncRepo"];

const runKind = (job: Job): SegmentKind =>
  EXPLORE_TYPES.includes(job.type) ? "explore" : SHIP_TYPES.includes(job.type) ? "ship" : "implement";

const parse = (iso: string | undefined): number | null => {
  if (!iso) return null;
  const at = Date.parse(iso);
  return Number.isNaN(at) ? null : at;
};

/**
 * One lane per job that is active or ended inside the window, running first, newest first. A stalled
 * job's run stops at its last output and the rest of the lane is the stall.
 */
export function buildFleet(jobs: readonly Job[], now: number = Date.now()): FleetLane[] {
  const windowStart = now - FLEET_WINDOW_MINUTES * MINUTE_MS;
  const pct = (at: number) =>
    Math.max(0, Math.min(100, ((at - windowStart) / (FLEET_WINDOW_MINUTES * MINUTE_MS)) * 100));

  const lanes: FleetLane[] = [];
  for (const job of jobs) {
    const active = ACTIVE_JOB_STATUSES.includes(job.status);
    const ended = parse(job.completedAt);
    if (!active && (ended == null || ended < windowStart)) continue;

    const started = parse(job.startedAt);
    if (job.status === "Queued" || job.status === "Pending" || started == null) {
      if (active) lanes.push({ job, segments: [], state: "queued", stalledFor: null });
      continue;
    }

    const from = pct(started);
    if (!active) {
      const failed = job.status === "Failed" || job.status === "Timeout";
      lanes.push({
        job,
        segments: [{ kind: failed ? "failed" : "done", from, to: pct(ended ?? now) }],
        state: failed ? "failed" : "done",
        stalledFor: null,
      });
      continue;
    }

    const stalled = stalledMinutes(job, now);
    if (stalled != null) {
      const lastOutput = parse(job.lastOutputAt) ?? started;
      const segments: FleetSegment[] = [];
      if (lastOutput > started) segments.push({ kind: runKind(job), from, to: pct(lastOutput) });
      segments.push({ kind: "stalled", from: pct(lastOutput), to: 100 });
      lanes.push({ job, segments, state: "stalled", stalledFor: stalled });
    } else {
      lanes.push({ job, segments: [{ kind: runKind(job), from, to: 100 }], state: "running", stalledFor: null });
    }
  }

  const rank = { running: 0, stalled: 1, queued: 2, failed: 3, done: 4 } as const;
  return lanes
    .sort(
      (a, b) =>
        rank[a.state] - rank[b.state] ||
        (parse(b.job.startedAt) ?? 0) - (parse(a.job.startedAt) ?? 0),
    )
    .slice(0, FLEET_LANES);
}

/** The five axis labels across the window: four clock times and "now". */
export function fleetAxis(now: number = Date.now()): string[] {
  const fmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
  return [0, 1, 2, 3].map((i) =>
    fmt.format(new Date(now - (FLEET_WINDOW_MINUTES - i * (FLEET_WINDOW_MINUTES / 4)) * MINUTE_MS)),
  );
}

export interface FleetStats {
  running: number;
  queued: number;
  tokensInFlight: number;
  spendInFlight: number;
  /** Median duration of jobs completed in the last 7 days, in seconds; null without any. */
  medianSeconds: number | null;
}

export function buildFleetStats(jobs: readonly Job[], now: number = Date.now()): FleetStats {
  const active = jobs.filter((j) => ACTIVE_JOB_STATUSES.includes(j.status));
  const durations = jobs
    .filter((j) => j.status === "Completed" && (parse(j.completedAt) ?? 0) >= now - 7 * DAY_MS)
    .map((j) => {
      if (j.durationSeconds != null) return j.durationSeconds;
      const s = parse(j.startedAt);
      const e = parse(j.completedAt);
      return s != null && e != null ? (e - s) / 1000 : null;
    })
    .filter((d): d is number => d != null && d > 0)
    .sort((a, b) => a - b);
  return {
    running: active.filter((j) => j.status === "Running").length,
    queued: active.filter((j) => j.status === "Queued" || j.status === "Pending").length,
    tokensInFlight: active.reduce((sum, j) => sum + (j.tokens ?? 0), 0),
    spendInFlight: active.reduce((sum, j) => sum + (j.cost ?? 0), 0),
    medianSeconds: durations.length ? durations[Math.floor(durations.length / 2)] : null,
  };
}

/** 18m 40s / 1h 12m. */
export const formatDuration = (seconds: number): string => {
  const s = Math.round(seconds);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
};

export type DecisionKind = "review" | "failed" | "stalled" | "draft";

export interface Decision {
  kind: DecisionKind;
  /** `plan:<id>` or `job:<id>`, what a click opens. */
  target: string;
  planId?: string;
  title: string;
  /** Milliseconds since the item started waiting, when known. */
  waitingMs: number | null;
  checks?: { passed: number; total: number };
  stalledFor?: number;
  level?: string;
  project?: string;
}

export const DECISIONS_LIMIT = 5;

/**
 * What waits on the operator, in the order they would clear it: plans ready for review, failed plans,
 * stalled jobs, then drafts that have not been executed. A plan an unfinished job still holds is not
 * offered, since its state is about to change under the operator's hand.
 */
export function buildDecisions(
  plans: readonly PlanSummary[],
  jobs: readonly Job[],
  now: number = Date.now(),
): { items: Decision[]; total: number } {
  const held = new Set(
    jobs
      .filter((j) => ACTIVE_JOB_STATUSES.includes(j.status))
      .map((j) => j.planId)
      .filter(Boolean),
  );
  const waited = (plan: PlanSummary) => {
    const at = parse(plan.updated ?? plan.created);
    return at == null ? null : now - at;
  };
  const oldestFirst = (a: Decision, b: Decision) => (b.waitingMs ?? 0) - (a.waitingMs ?? 0);
  const fromPlan = (plan: PlanSummary, kind: DecisionKind): Decision => {
    const checks = plan.verifications.filter((v) => v.status !== "Skipped");
    return {
      kind,
      target: `plan:${plan.id}`,
      planId: plan.id,
      title: plan.title,
      waitingMs: waited(plan),
      checks: checks.length
        ? { passed: checks.filter((v) => v.status === "Pass").length, total: checks.length }
        : undefined,
      level: plan.level,
      project: plan.project,
    };
  };
  const inState = (state: string) => plans.filter((p) => p.state === state && !held.has(p.id));

  const review = inState("Review").map((p) => fromPlan(p, "review")).sort(oldestFirst);
  const failed = inState("Failed").map((p) => fromPlan(p, "failed")).sort(oldestFirst);
  const stalled = jobs.flatMap((job): Decision[] => {
    const idle = stalledMinutes(job, now);
    return idle == null
      ? []
      : [
          {
            kind: "stalled",
            target: `job:${job.id}`,
            planId: job.planId,
            title: job.planTitle ?? job.project,
            waitingMs: idle * MINUTE_MS,
            stalledFor: idle,
          },
        ];
  });
  const drafts = inState("Draft").map((p) => fromPlan(p, "draft")).sort(oldestFirst);

  const all = [...review, ...failed, ...stalled, ...drafts];
  return { items: all.slice(0, DECISIONS_LIMIT), total: all.length };
}

export type ActivityKind = "merged" | "finished" | "failed" | "drafted" | "pr";

export interface ActivityEvent {
  kind: ActivityKind;
  at: number;
  planId?: string;
  title: string;
  detail?: string;
  target?: string;
}

/** Recent job endings and merged PRs, newest first. */
export function buildActivity(
  jobs: readonly Job[],
  mergedPrs: readonly RecentMergedPr[],
  limit = 6,
): ActivityEvent[] {
  const events: ActivityEvent[] = [];
  for (const job of jobs) {
    const at = parse(job.completedAt);
    if (at == null) continue;
    const title = job.planTitle ?? job.prompt ?? job.project;
    if (job.status === "Failed" || job.status === "Timeout") {
      events.push({ kind: "failed", at, planId: job.planId, title, detail: job.statusMessage ?? job.type, target: `job:${job.id}` });
    } else if (job.status === "Completed") {
      const kind: ActivityKind =
        job.type === "CreatePlan" ? "drafted" : job.type === "CreatePr" ? "pr" : "finished";
      events.push({ kind, at, planId: job.planId, title, detail: job.type, target: job.planId ? `plan:${job.planId}` : `job:${job.id}` });
    }
  }
  for (const pr of mergedPrs) {
    const at = parse(pr.updated);
    if (at == null) continue;
    events.push({ kind: "merged", at, planId: String(pr.planId), title: pr.title, detail: pr.repo ?? pr.prUrl, target: `plan:${pr.planId}` });
  }
  return events.sort((a, b) => b.at - a.at).slice(0, limit);
}

export type HeatMetric = "tokens" | "cost" | "plans";

export interface HeatDay {
  date: string;
  value: number;
}

/** `weeks` × 7 days ending today, zero-filled, from the analytics daily series. */
export function buildHeatDays(
  activity: DashboardActivity | null,
  metric: HeatMetric,
  weeks: number,
  today: number = todayDayNumber(),
): HeatDay[] {
  const byDay = new Map<string, number>();
  if (metric === "plans") {
    for (const d of activity?.dailyPlans ?? []) byDay.set(d.date, d.count);
  } else {
    for (const d of activity?.dailyCosts ?? []) byDay.set(d.date, metric === "tokens" ? d.tokens : d.cost);
  }
  // End on the last day of this week so columns are whole weeks (Sunday first).
  const todayDow = new Date(toIsoDate(today) + "T00:00:00").getDay();
  const end = today + (6 - todayDow);
  const days = weeks * 7;
  return Array.from({ length: days }, (_, i) => {
    const dayNumber = end - days + 1 + i;
    const date = toIsoDate(dayNumber);
    return { date, value: dayNumber > today ? -1 : (byDay.get(date) ?? 0) };
  });
}

export interface HeatSummary {
  total: number;
  avg: number;
  peak: HeatDay | null;
  streak: number;
  longestStreak: number;
  busiestWeekday: number | null;
}

export function summarizeHeat(days: readonly HeatDay[]): HeatSummary {
  const past = days.filter((d) => d.value >= 0);
  const total = past.reduce((s, d) => s + d.value, 0);
  const active = past.filter((d) => d.value > 0);
  let peak: HeatDay | null = null;
  for (const d of past) if (d.value > 0 && (peak == null || d.value > peak.value)) peak = d;

  let streak = 0;
  for (let i = past.length - 1; i >= 0; i--) {
    if (past[i].value > 0) streak++;
    else if (i === past.length - 1) continue; // today may not have started yet
    else break;
  }
  let longest = 0;
  let run = 0;
  for (const d of past) {
    run = d.value > 0 ? run + 1 : 0;
    longest = Math.max(longest, run);
  }
  const byDow = [0, 0, 0, 0, 0, 0, 0];
  for (const d of past) byDow[new Date(d.date + "T00:00:00").getDay()] += d.value;
  const busiest = total > 0 ? byDow.indexOf(Math.max(...byDow)) : null;

  return {
    total,
    avg: active.length ? total / past.length : 0,
    peak,
    streak,
    longestStreak: longest,
    busiestWeekday: busiest,
  };
}

/** The last `n` days of a daily series ending today, zero-filled. */
export function lastDays(
  activity: DashboardActivity | null,
  pick: "tokens" | "cost",
  n: number,
  today: number = todayDayNumber(),
): number[] {
  const byDay = new Map((activity?.dailyCosts ?? []).map((d) => [d.date, d[pick]]));
  return Array.from({ length: n }, (_, i) => byDay.get(toIsoDate(today - n + 1 + i)) ?? 0);
}
