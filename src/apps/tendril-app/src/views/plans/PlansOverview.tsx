import React from "react";
import { Plus, Search } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { Job, PlanSummary } from "../../types/api";
import { useTranslation } from "../../i18n";
import { useEnumLabels } from "../../i18n/enumLabels";
import { Pill, PrimaryButton, Seg, modKey, type Tone } from "../../components/page/kit";
import { formatAge } from "../../utils/commandCenter";
import { normalizePlanState } from "../../utils/planQueues";
import { ACTIVE_JOB_STATUSES } from "../../utils/processStatus";

type Filter = "queue" | "active" | "review" | "done" | "all";

const FILTER_STATES: Record<Exclude<Filter, "all">, string[]> = {
  queue: ["Draft", "Blocked"],
  active: ["Creating", "Updating", "Executing"],
  review: ["Review", "Failed"],
  done: ["Completed", "Skipped"],
};

const STATE_TONE: Record<string, Tone> = {
  Draft: "mute",
  Blocked: "warn",
  Creating: "info",
  Updating: "info",
  Executing: "ok",
  Review: "info",
  Failed: "bad",
  Completed: "violet",
  Skipped: "mute",
  Icebox: "mute",
};

const updatedAt = (p: PlanSummary) => Date.parse(p.updated ?? p.created ?? "") || 0;

/**
 * The Plans page: every plan outside a mission, filterable by where it is in its life, newest first.
 * A row opens the plan. A plan with a job running on it shows the pulse, so what is moving stands out.
 */
export const PlansOverview: React.FC<{
  plans: PlanSummary[];
  jobs: Job[];
  onSelectPlan: (planId: string) => void;
  onNewPlan?: () => void;
}> = ({ plans, jobs, onSelectPlan, onNewPlan }) => {
  const { t } = useTranslation("plans");
  const labels = useEnumLabels();
  const [filter, setFilter] = React.useState<Filter>("queue");
  const [query, setQuery] = React.useState("");

  const visible = plans.filter((p) => normalizePlanState(p.state) !== "Icebox");
  const running = new Set(
    jobs.filter((j) => ACTIVE_JOB_STATUSES.includes(j.status)).map((j) => j.planId).filter(Boolean),
  );
  const count = (f: Filter) =>
    f === "all" ? visible.length : visible.filter((p) => FILTER_STATES[f].includes(normalizePlanState(p.state))).length;

  const q = query.trim().toLowerCase();
  const rows = visible
    .filter((p) => filter === "all" || FILTER_STATES[filter].includes(normalizePlanState(p.state)))
    .filter((p) => !q || `${p.id} ${p.title} ${p.project} ${p.level}`.toLowerCase().includes(q))
    .sort((a, b) => updatedAt(b) - updatedAt(a));

  const now = Date.now();
  const cols = "grid grid-cols-[64px_minmax(0,1fr)_minmax(0,160px)_96px_120px_72px_64px] gap-3";

  return (
    <div data-testid="plans-overview" className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-6 pt-5">
      <header className="flex flex-wrap items-end gap-3">
        <div className="flex flex-col gap-1.5">
          <h1 className="m-0 text-[26px] font-semibold tracking-[-0.025em]">{t("overview.title")}</h1>
          <p className="m-0 text-[13px] text-muted-foreground">
            {t("overview.subtitle", { queue: count("queue"), active: count("active"), review: count("review") })}
          </p>
        </div>
        {onNewPlan && (
          <PrimaryButton className="ml-auto" onClick={onNewPlan}>
            <Plus size={14} aria-hidden="true" />
            {t("overview.new")}
            <kbd className="ml-1 font-mono text-[10.5px] opacity-70">{modKey()}N</kbd>
          </PrimaryButton>
        )}
      </header>

      <div className="flex flex-wrap items-center gap-2">
        <Seg<Filter>
          label={t("overview.title")}
          value={filter}
          onChange={setFilter}
          options={(["queue", "active", "review", "done", "all"] as const).map((f) => ({
            value: f,
            label: (
              <>
                {t(`overview.filter.${f}`)}
                <span className="font-mono text-[10.5px] text-muted-foreground">{count(f)}</span>
              </>
            ),
          }))}
        />
        <label className="ml-auto flex h-8 w-72 items-center gap-2 rounded-lg border border-border bg-card px-2.5 text-muted-foreground focus-within:border-input">
          <Search size={14} aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("overview.search")}
            aria-label={t("overview.search")}
            className="min-w-0 flex-1 bg-transparent text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground"
          />
        </label>
      </div>

      <section className="overflow-hidden rounded-xl border border-border bg-card">
        <div className={cn(cols, "px-4 pb-2 pt-3 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground")}>
          <span>{t("overview.columns.id")}</span>
          <span>{t("overview.columns.plan")}</span>
          <span>{t("overview.columns.project")}</span>
          <span>{t("overview.columns.level")}</span>
          <span>{t("overview.columns.state")}</span>
          <span className="text-right">{t("overview.columns.checks")}</span>
          <span className="text-right">{t("overview.columns.updated")}</span>
        </div>
        {rows.length === 0 && (
          <p className="m-0 border-t border-border/70 px-4 py-10 text-center text-[13px] text-muted-foreground">
            {q ? t("overview.noMatch") : t("overview.emptyFilter")}
          </p>
        )}
        {rows.map((p) => {
          const state = normalizePlanState(p.state);
          const checks = p.verifications.filter((v) => v.status !== "Skipped");
          const passed = checks.filter((v) => v.status === "Pass").length;
          const failed = checks.some((v) => v.status === "Fail");
          const live = running.has(p.id);
          return (
            <button
              key={p.id}
              type="button"
              onClick={() => onSelectPlan(p.id)}
              className={cn(cols, "h-11 w-full items-center border-t border-border/70 px-4 text-left text-[13px] transition-colors hover:bg-muted/60")}
            >
              <span className="flex items-center gap-1.5 font-mono text-[11.5px] text-muted-foreground">
                {live && <span className="size-1.5 animate-pulse rounded-full bg-success" aria-hidden="true" />}#{p.id}
              </span>
              <span className="truncate font-medium">{p.title}</span>
              <span className="truncate text-xs text-muted-foreground">{p.project}</span>
              <span className="truncate text-xs text-muted-foreground">{p.level}</span>
              <span>
                <Pill tone={STATE_TONE[state] ?? "mute"} dot={live} live={live}>
                  {labels.planState(state)}
                </Pill>
              </span>
              <span
                className={cn(
                  "text-right font-mono text-[11.5px]",
                  failed ? "text-destructive" : checks.length && passed === checks.length ? "text-success" : "text-muted-foreground",
                )}
              >
                {checks.length ? `${passed}/${checks.length}` : "—"}
              </span>
              <span className="text-right font-mono text-[11px] text-muted-foreground">
                {updatedAt(p) ? formatAge(now - updatedAt(p)) : "—"}
              </span>
            </button>
          );
        })}
      </section>
    </div>
  );
};
