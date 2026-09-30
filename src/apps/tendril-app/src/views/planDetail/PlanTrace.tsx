import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import { useEnumLabels } from "../../i18n/enumLabels";
import type { Job } from "../../types/api";
import { AgentAvatar, Card, Pill, type Tone } from "../../components/page/kit";
import { ACTIVE_JOB_STATUSES } from "../../utils/processStatus";
import { formatTokens, stalledMinutes } from "../../utils/commandCenter";
import { formatCurrency } from "../../utils/dashboardMetrics";
import { formatDuration } from "../../utils/fleet";

const EXPLORE = ["CreatePlan", "UpdatePlan", "ExpandPlan", "SplitPlan", "CreateIssue"];
const SHIP = ["CreatePr", "SyncRepo"];

const barStyle = (job: Job, now: number): React.CSSProperties => {
  if (job.status === "Failed" || job.status === "Timeout") return { background: "#e25c66" };
  if (job.status === "Stopped") return { background: "#3a4756" };
  if (job.status === "Running" && stalledMinutes(job, now) != null)
    return {
      background: "repeating-linear-gradient(135deg,rgba(240,181,74,.6) 0 4px,rgba(240,181,74,.2) 4px 8px)",
    };
  const running = job.status === "Running";
  if (EXPLORE.includes(job.type)) return { background: running ? "#2f4256" : "#1f2c39", boxShadow: "inset 0 0 0 1px #2e3d4e" };
  if (SHIP.includes(job.type)) return { background: "#4f86e6" };
  return running
    ? { background: "linear-gradient(180deg,#19e0a5,#00b582)", boxShadow: "0 0 12px rgba(0,204,146,.35)" }
    : { background: "#0b8a64" };
};

const STATUS_TONE: Record<string, Tone> = {
  Running: "ok",
  Completed: "mute",
  Failed: "bad",
  Timeout: "bad",
  Stopped: "mute",
  Queued: "info",
  Pending: "info",
  Blocked: "warn",
};

/**
 * Every run the plan has had, as a waterfall on one time axis from its first job to now: which agent,
 * what it did, how long, and what it cost. A row opens that job's debug sheet.
 */
export const PlanTrace: React.FC<{
  jobs: Job[];
  planId: string;
  onOpenJob: (jobId: string) => void;
}> = ({ jobs, planId, onOpenJob }) => {
  const { t } = useTranslation("plans");
  const labels = useEnumLabels();
  const now = Date.now();

  const runs = jobs
    .filter((j) => j.planId === planId)
    .map((j) => {
      const start = Date.parse(j.startedAt ?? "");
      const endRaw = Date.parse(j.completedAt ?? "");
      const active = ACTIVE_JOB_STATUSES.includes(j.status);
      return {
        job: j,
        start: Number.isNaN(start) ? null : start,
        end: active || Number.isNaN(endRaw) ? now : endRaw,
      };
    })
    .sort((a, b) => (a.start ?? Number.MAX_SAFE_INTEGER) - (b.start ?? Number.MAX_SAFE_INTEGER));

  const started = runs.filter((r) => r.start != null);
  const t0 = started.length ? Math.min(...started.map((r) => r.start!)) : now;
  const t1 = Math.max(now, ...runs.map((r) => r.end));
  const span = Math.max(1, t1 - t0);
  const totalTokens = runs.reduce((s, r) => s + (r.job.tokens ?? 0), 0);
  const totalCost = runs.reduce((s, r) => s + (r.job.cost ?? 0), 0);
  const agentSeconds = runs.reduce((s, r) => s + (r.start != null ? (r.end - r.start) / 1000 : 0), 0);

  const tickFmt = new Intl.DateTimeFormat(undefined, span > 36 * 3600_000 ? { month: "short", day: "numeric" } : { hour: "2-digit", minute: "2-digit" });
  const ticks = [0, 0.25, 0.5, 0.75, 1].map((f) => tickFmt.format(new Date(t0 + span * f)));
  const cols = "grid grid-cols-[minmax(0,260px)_minmax(0,1fr)_64px_56px_60px] gap-3";

  return (
    <div data-testid="plan-trace" className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 py-5">
      <Card
        title={t("detail.trace.title")}
        meta={
          <span className="font-mono text-[11px] text-muted-foreground">
            {t("detail.trace.summary", {
              runs: String(runs.length),
              time: agentSeconds > 0 ? formatDuration(agentSeconds) : "—",
              tokens: formatTokens(totalTokens) ?? "0",
              cost: formatCurrency(totalCost),
            })}
          </span>
        }
      >
        {runs.length === 0 ? (
          <p className="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">{t("detail.trace.empty")}</p>
        ) : (
          <>
            <div className={cn(cols, "px-4 pb-1.5 pt-2.5 font-mono text-[10px] uppercase text-muted-foreground")}>
              <span>{t("detail.trace.run")}</span>
              <span className="flex justify-between normal-case">
                {ticks.map((tick, i) => (
                  <span key={i}>{tick}</span>
                ))}
              </span>
              <span className="text-right">{t("detail.trace.time")}</span>
              <span className="text-right">{t("detail.trace.tokens")}</span>
              <span className="text-right">{t("detail.trace.cost")}</span>
            </div>
            {runs.map(({ job, start, end }) => {
              const left = start != null ? ((start - t0) / span) * 100 : 100;
              const width = start != null ? ((end - start) / span) * 100 : 0;
              const running = job.status === "Running";
              return (
                <button
                  key={job.id}
                  type="button"
                  onClick={() => onOpenJob(job.id)}
                  className={cn(
                    cols,
                    "h-9 w-full items-center border-t border-border/70 px-4 text-left transition-colors hover:bg-muted/50",
                    running && "bg-primary/[0.04]",
                  )}
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <AgentAvatar agent={job.model} size={18} />
                    <span className="truncate font-mono text-[11.5px] font-medium">{labels.jobType(job.type)}</span>
                    <Pill tone={STATUS_TONE[job.status] ?? "mute"} className="h-4 px-1.5 text-[10px]">
                      {labels.jobStatus(job.status)}
                    </Pill>
                  </span>
                  <span
                    className="relative h-9"
                    style={{
                      backgroundImage: "linear-gradient(90deg,var(--border) 1px,transparent 1px)",
                      backgroundSize: "25% 100%",
                    }}
                  >
                    {start == null ? (
                      <span className="absolute bottom-2.5 right-0 top-2.5 w-10 rounded border border-dashed border-input" />
                    ) : (
                      <span
                        className="absolute bottom-[11px] top-[11px] rounded"
                        style={{ left: `${left}%`, width: `max(3px, ${width}%)`, ...barStyle(job, now) }}
                      />
                    )}
                  </span>
                  <span className="text-right font-mono text-[11px] text-muted-foreground">
                    {start != null ? formatDuration((end - start) / 1000) : "—"}
                  </span>
                  <span className="text-right font-mono text-[11px]">{formatTokens(job.tokens) ?? "—"}</span>
                  <span className="text-right font-mono text-[11px] text-muted-foreground">
                    {job.cost != null ? formatCurrency(job.cost) : "—"}
                  </span>
                </button>
              );
            })}
          </>
        )}
      </Card>
    </div>
  );
};
