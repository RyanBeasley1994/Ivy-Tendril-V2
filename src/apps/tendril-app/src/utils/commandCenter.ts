import type { Job } from "../types/api";

/**
 * Small formatting and job-health rules the command center, Insights, Review and the plan trace
 * share. The panels' own derived data lives in `fleet.ts`.
 */

/** Minutes without output before a running job is called stalled: `staleOutputTimeout`'s default. */
export const STALL_MINUTES = 10;

const MINUTE_MS = 60_000;

const minutesSince = (iso: string | undefined, now: number): number | null => {
  if (!iso) return null;
  const at = Date.parse(iso);
  return Number.isNaN(at) ? null : Math.floor((now - at) / MINUTE_MS);
};

/** A running job whose last output is older than {@link STALL_MINUTES}. */
export const stalledMinutes = (job: Job, now: number = Date.now()): number | null => {
  if (job.status !== "Running") return null;
  const idle = minutesSince(job.lastOutputAt ?? job.startedAt, now);
  return idle != null && idle >= STALL_MINUTES ? idle : null;
};

/** Token counts the way the panels print them: 912, 41k, 1.2M. */
export const formatTokens = (tokens: number | undefined): string | null => {
  if (tokens == null || !Number.isFinite(tokens)) return null;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}k`;
  return String(Math.round(tokens));
};

/** A duration the way a person says it: 45m, 3h, 2d. */
export const formatAge = (ms: number): string => {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
};
