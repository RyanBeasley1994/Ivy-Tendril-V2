import React, { useCallback, useEffect, useState } from "react";
import {
  Ban,
  Check,
  CircleDashed,
  ExternalLink,
  GitBranch,
  Pause,
  Play,
  Plus,
  Rocket,
  ThumbsUp,
  Eye,
  LayoutGrid,
  CircleCheck,
  ChevronRight,
  MoreHorizontal,
} from "lucide-react";
import { AgentPicker } from "@ivy-interactive/components/dialogs";
import { PlanMarkdown } from "@ivy-interactive/components/tendril";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Spinner,
  cn,
} from "@ivy-interactive/components/ui";
import {
  Card,
  GhostButton,
  Pill,
  PrimaryButton,
  type Tone,
} from "../components/page/kit";
import { ErrorBanner } from "../components/ErrorBanner";
import { bridge } from "../api/bridge";
import { uiStore } from "../state/uiStore";
import { useNavigation } from "../state/navigation";
import { useTranslation, type TFunction } from "../i18n";
import { visibleAgents } from "./settings/codingAgents";
import { formatAge } from "../utils/commandCenter";
import {
  describeBridgeError,
  type Milestone,
  type MilestoneState,
  type Mission,
  type MissionAction,
  type MissionAgents,
  type MissionRoleName,
  type MissionState,
  type TendrilConfig,
} from "../types/api";

/** How often the page re-reads missions while it is open. Missions move on job transitions, which
 *  take seconds to minutes; this is frequent enough to feel live without hammering the daemon. */
const POLL_MS = 4000;

const ROLES: MissionRoleName[] = ["planner", "worker", "judge", "validator"];

const STATE_TONE: Record<MissionState, Tone> = {
  Planning: "info",
  AwaitingApproval: "warn",
  Running: "ok",
  Validating: "violet",
  Review: "info",
  Completed: "ok",
  Paused: "warn",
  Cancelled: "mute",
};

const MILESTONE_TONE: Record<MilestoneState, Tone> = {
  Pending: "mute",
  Executing: "ok",
  Judging: "violet",
  Passed: "ok",
  Skipped: "mute",
};

/** The milestone's segment colour in a progress bar. */
const SEGMENT: Record<MilestoneState, string> = {
  Pending: "bg-secondary",
  Executing: "bg-primary/60 animate-pulse",
  Judging: "bg-violet/70 animate-pulse",
  Passed: "bg-primary",
  Skipped: "bg-secondary/50",
};

/** The mission's five phases, and which one each state sits in. */
const PHASES = ["plan", "approve", "run", "validate", "review"] as const;
const PHASE_OF: Partial<Record<MissionState, number>> = {
  Planning: 0,
  AwaitingApproval: 1,
  Running: 2,
  Validating: 3,
  Review: 4,
  Completed: 5,
};

/** The mission is doing something on its own right now: a job runs or is about to. */
const isLive = (state: MissionState) =>
  state === "Planning" || state === "Running" || state === "Validating";

const isTerminal = (state: MissionState) =>
  state === "Completed" || state === "Cancelled";

const money = (value: number) => `$${value.toFixed(2)}`;


/** The plan id (`00042`) at the front of a plan folder name (`00042-AddLogin`). */
const planIdOf = (folder: string) => folder.slice(0, 5);

/** The list's filters, and which states each keeps. */
type ListFilter = "progress" | "review" | "done" | "all";
const LIST_FILTER: Record<Exclude<ListFilter, "all">, MissionState[]> = {
  progress: ["Planning", "AwaitingApproval", "Running", "Validating", "Paused"],
  review: ["Review"],
  done: ["Completed", "Cancelled"],
};
const inFilter = (m: Mission, f: ListFilter) =>
  f === "all" || LIST_FILTER[f].includes(m.state);

/** The list's "every live mission" entry, standing in for a mission id in the page args. */
const OVERVIEW_ID = "all";

const sinceIso = (iso: string) => {
  const at = Date.parse(iso);
  return Number.isNaN(at) ? "" : formatAge(Date.now() - at);
};

export interface MissionsViewProps {
  onSelectPlan: (planId: string) => void;
  onSelectJob: (jobId: string) => void;
  /** Opens Create New Plan in Mission mode. */
  onNewMission: () => void;
}

/**
 * Missions: goals an AI orchestrator breaks into milestones and runs on its own. Mission control:
 * the missions down the left, the selected one on the right - where it is in its five phases, its
 * milestones as a track, the crew each role runs on, the goal and the orchestrator's log. The only
 * decision the operator has to make is the approval; everything else is steering and following.
 */
export const MissionsView: React.FC<MissionsViewProps> = ({
  onSelectPlan,
  onSelectJob,
  onNewMission,
}) => {
  const { t } = useTranslation("missions");
  const { pageArgs } = useNavigation();
  const [missions, setMissions] = useState<Mission[] | null>(null);
  const [detail, setDetail] = useState<Mission | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  /** For the harnesses Settings hides from the agent pickers. */
  const [config, setConfig] = useState<TendrilConfig | null>(null);

  useEffect(() => {
    void Promise.resolve()
      .then(() => bridge.getConfig())
      .then((c) => setConfig(c ?? null))
      .catch(() => setConfig(null));
  }, []);

  const liveCount = (missions ?? []).filter((m) => !isTerminal(m.state)).length;
  // Two or more missions in flight open on the board of all of them; otherwise on the first mission.
  const selectedId =
    pageArgs.mission ??
    (liveCount > 1 ? OVERVIEW_ID : (missions?.[0]?.id ?? null));
  const select = (id: string) =>
    uiStore.setActiveNav("missions", { mission: id });

  const refresh = useCallback(async () => {
    try {
      const list = await bridge.listMissions();
      setMissions(list);
      const live = list.filter((m) => !isTerminal(m.state)).length;
      const id = pageArgs.mission ?? (live > 1 ? OVERVIEW_ID : list[0]?.id);
      setDetail(id && id !== OVERVIEW_ID ? await bridge.getMission(id) : null);
      setError(null);
    } catch (err) {
      setError(describeBridgeError(err));
      setMissions((current) => current ?? []);
    }
  }, [pageArgs.mission]);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const act = async (action: MissionAction) => {
    if (!detail) return;
    setBusy(action);
    try {
      setDetail(await bridge.missionAction(detail.id, action));
      void refresh();
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setBusy(null);
    }
  };

  /** An action on any mission in the list (its right-click menu), not only the one on screen. */
  const actOn = async (id: string, action: MissionAction) => {
    setBusy(action);
    try {
      const updated = await bridge.missionAction(id, action);
      if (detail?.id === id) setDetail(updated);
      void refresh();
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setBusy(null);
    }
  };

  const changeAgent = async (role: MissionRoleName, agent: string) => {
    if (!detail) return;
    const next: MissionAgents = { ...detail.agents };
    if (agent) next[role] = { agent };
    else delete next[role];
    setBusy(`agent-${role}`);
    try {
      setDetail(await bridge.setMissionAgents(detail.id, next));
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setBusy(null);
    }
  };

  const shown = detail && detail.id === selectedId ? detail : null;

  if (missions === null) {
    return (
      <div
        className="flex flex-1 items-center justify-center"
        data-testid="missions-view"
      >
        <Spinner />
      </div>
    );
  }

  if (missions.length === 0) {
    return (
      <div
        className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 py-5"
        data-testid="missions-view"
      >
        {error && (
          <ErrorBanner onDismiss={() => setError(null)}>{error}</ErrorBanner>
        )}
        <EmptyState t={t} onNewMission={onNewMission} />
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1" data-testid="missions-view">
      <MissionList
        missions={missions}
        selectedId={selectedId}
        onSelect={select}
        onNewMission={onNewMission}
        onAction={(id, action) => void actOn(id, action)}
        onSelectJob={onSelectJob}
        t={t}
      />
      <main className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 pb-8 pt-5">
        {error && (
          <ErrorBanner onDismiss={() => setError(null)}>{error}</ErrorBanner>
        )}
        {selectedId === OVERVIEW_ID ? (
          <MissionsOverview
            missions={missions}
            t={t}
            onOpen={select}
            onSelectJob={onSelectJob}
          />
        ) : shown ? (
          <MissionDetail
            mission={shown}
            config={config}
            busy={busy}
            t={t}
            onAction={(a) => void act(a)}
            onChangeAgent={(role, agent) => void changeAgent(role, agent)}
            onSelectPlan={onSelectPlan}
            onSelectJob={onSelectJob}
          />
        ) : (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        )}
      </main>
    </div>
  );
};

/* ---------------------------------------------------------------- empty */

const EmptyState: React.FC<{
  t: TFunction<"missions">;
  onNewMission: () => void;
}> = ({ t, onNewMission }) => (
  <div
    className="mx-auto mt-10 flex w-full max-w-3xl flex-col gap-6"
    data-testid="missions-empty"
  >
    <div className="flex flex-col items-start gap-3">
      <span className="flex size-11 items-center justify-center rounded-xl bg-primary/12 text-success shadow-[0_10px_30px_-12px_rgba(0,204,146,0.6)]">
        <Rocket size={20} aria-hidden="true" />
      </span>
      <h1 className="m-0 text-[26px] font-semibold tracking-[-0.025em]">
        {t("empty.title")}
      </h1>
      <p className="m-0 max-w-xl text-[13.5px] leading-relaxed text-muted-foreground">
        {t("empty.description")}
      </p>
    </div>
    <ol className="m-0 grid list-none grid-cols-1 gap-0 overflow-hidden rounded-xl border border-border bg-card p-0 sm:grid-cols-4">
      {(["plan", "approve", "run", "ship"] as const).map((step, index) => (
        <li
          key={step}
          className={cn(
            "flex flex-col gap-2 p-4",
            index > 0 && "sm:border-l sm:border-border/70",
          )}
        >
          <span className="flex items-center gap-2">
            <span className="flex size-5 items-center justify-center rounded-full bg-secondary font-mono text-[10.5px] text-muted-foreground">
              {index + 1}
            </span>
            <span className="text-[13px] font-medium">
              {t(`empty.steps.${step}.title`)}
            </span>
          </span>
          <span className="text-[12px] leading-snug text-muted-foreground">
            {t(`empty.steps.${step}.body`)}
          </span>
        </li>
      ))}
    </ol>
    <div>
      <PrimaryButton size="lg" onClick={onNewMission} data-testid="new-mission">
        <Plus size={14} aria-hidden="true" />
        {t("page.new")}
      </PrimaryButton>
    </div>
  </div>
);

/* ----------------------------------------------------------------- list */

const Segments: React.FC<{ milestones: Milestone[]; height?: number }> = ({
  milestones,
  height = 4,
}) =>
  milestones.length === 0 ? (
    <span className="block rounded-full bg-secondary" style={{ height }} />
  ) : (
    <span className="flex gap-[3px]" style={{ height }} aria-hidden="true">
      {milestones.map((m) => (
        <span
          key={m.id}
          className={cn("flex-1 rounded-full", SEGMENT[m.state])}
        />
      ))}
    </span>
  );

const MissionList: React.FC<{
  missions: Mission[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onNewMission: () => void;
  onAction: (id: string, action: MissionAction) => void;
  onSelectJob: (jobId: string) => void;
  t: TFunction<"missions">;
}> = ({
  missions,
  selectedId,
  onSelect,
  onNewMission,
  onAction,
  onSelectJob,
  t,
}) => {
  const [filter, setFilter] = useState<ListFilter>(() => {
    try {
      const saved = localStorage.getItem("tendril.missions.filter");
      return saved === "review" || saved === "done" || saved === "all"
        ? saved
        : "progress";
    } catch {
      return "progress";
    }
  });
  const pick = (f: ListFilter) => {
    setFilter(f);
    try {
      localStorage.setItem("tendril.missions.filter", f);
    } catch {
      /* per-viewer convenience only */
    }
  };
  const shownMissions = missions.filter((m) => inFilter(m, filter));
  const liveCount = missions.filter((m) => !isTerminal(m.state)).length;
  const anyWorking = missions.some((m) => isLive(m.state));

  return (
    <aside
      aria-label={t("list.title")}
      data-testid="missions-list"
      className="flex w-[300px] shrink-0 flex-col border-r border-border/80 bg-[color-mix(in_srgb,var(--card)_45%,var(--background))]"
    >
      <div className="flex h-[52px] shrink-0 items-center gap-2 pl-4 pr-3">
        <h2 className="m-0 text-sm font-semibold">{t("list.title")}</h2>
        <span className="font-mono text-[11px] text-muted-foreground">
          {missions.length}
        </span>
        <PrimaryButton
          size="sm"
          className="ml-auto"
          onClick={onNewMission}
          data-testid="new-mission"
        >
          <Plus size={13} aria-hidden="true" />
          {t("page.newShort")}
        </PrimaryButton>
      </div>

      <div
        role="tablist"
        aria-label={t("filter.label")}
        className="flex shrink-0 gap-4 border-b border-border/80 px-4"
      >
        {(["progress", "review", "done", "all"] as const).map((f) => {
          const on = filter === f;
          const n = missions.filter((m) => inFilter(m, f)).length;
          return (
            <button
              key={f}
              type="button"
              role="tab"
              aria-selected={on}
              onClick={() => pick(f)}
              className={cn(
                "-mb-px flex h-9 items-center gap-1.5 whitespace-nowrap border-b-2 text-[12.5px] transition-colors",
                on
                  ? "border-primary font-medium text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground",
              )}
            >
              {t(`filter.${f}`)}
              {n > 0 && (
                <span className="font-mono text-[10px] text-muted-foreground">
                  {n}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <ul className="m-0 flex min-h-0 flex-1 list-none flex-col gap-1 overflow-y-auto p-2">
        {liveCount > 0 && (
          <li>
            <button
              type="button"
              onClick={() => onSelect(OVERVIEW_ID)}
              aria-current={selectedId === OVERVIEW_ID}
              className={cn(
                "flex h-9 w-full items-center gap-2.5 rounded-[10px] px-3 text-left text-[13px] transition-colors hover:bg-secondary/40",
                selectedId === OVERVIEW_ID &&
                  "bg-[#101820] shadow-[inset_0_0_0_1px_#1e3a33]",
              )}
            >
              <LayoutGrid
                size={14}
                className="text-muted-foreground"
                aria-hidden="true"
              />
              {t("overview.title")}
              <span className="ml-auto flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
                {anyWorking && (
                  <span
                    className="size-1.5 rounded-full bg-success shadow-[0_0_0_3px_rgba(0,204,146,0.2)]"
                    aria-hidden="true"
                  />
                )}
                {liveCount}
              </span>
            </button>
          </li>
        )}
        {shownMissions.length === 0 && (
          <li className="flex flex-col items-center gap-1 px-3 py-8 text-center">
            <span className="text-[12.5px] text-muted-foreground">
              {t(`filter.emptyFor.${filter}`)}
            </span>
            {filter !== "all" && (
              <button
                type="button"
                onClick={() => pick("all")}
                className="text-[12px] text-success hover:underline"
              >
                {t("filter.showAll")}
              </button>
            )}
          </li>
        )}
        {shownMissions.map((m) => {
          const passed = m.milestones.filter(
            (ms) => ms.state === "Passed",
          ).length;
          const total = m.milestones.filter(
            (ms) => ms.state !== "Skipped",
          ).length;
          const selected = m.id === selectedId;
          return (
            <li key={m.id}>
              <ContextMenu>
                <ContextMenuTrigger asChild>
                  <button
                    type="button"
                    onClick={() => onSelect(m.id)}
                    aria-current={selected}
                    className={cn(
                      "flex w-full flex-col gap-2 rounded-[10px] px-3 py-2.5 text-left transition-colors hover:bg-secondary/40",
                      selected &&
                        "bg-[#101820] shadow-[inset_0_0_0_1px_#1e3a33]",
                    )}
                  >
                    <span className="flex items-start gap-2">
                      <span className="min-w-0 flex-1 text-[13px] font-medium leading-snug">
                        {m.title}
                      </span>
                      <Pill
                        tone={STATE_TONE[m.state]}
                        dot
                        live={isLive(m.state)}
                        className="h-[18px] text-[10.5px]"
                      >
                        {t(`state.${m.state}`)}
                      </Pill>
                    </span>
                    {m.currentJob ? (
                      <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-success">
                        <span
                          className="size-1.5 shrink-0 animate-pulse rounded-full bg-success"
                          aria-hidden="true"
                        />
                        <span className="truncate">
                          {t(`step.${m.currentJob.step}`)}
                          {m.currentJob.milestone
                            ? ` ${m.currentJob.milestone}`
                            : ""}
                          {m.currentJob.agent
                            ? ` · ${m.currentJob.agent.split(" · ")[0]}`
                            : ""}
                        </span>
                      </span>
                    ) : (
                      !isTerminal(m.state) && (
                        <Segments milestones={m.milestones} height={3} />
                      )
                    )}
                    <span className="flex items-center gap-1.5 font-mono text-[10.5px] text-muted-foreground">
                      <span>#{m.id}</span>
                      <span className="truncate">· {m.project}</span>
                      <span className="ml-auto whitespace-nowrap">
                        {passed}/{total} · {money(m.cost)}
                      </span>
                    </span>
                  </button>
                </ContextMenuTrigger>
                <MissionMenu
                  mission={m}
                  t={t}
                  onOpen={() => onSelect(m.id)}
                  onAction={(a) => onAction(m.id, a)}
                  onSelectJob={onSelectJob}
                />
              </ContextMenu>
            </li>
          );
        })}
      </ul>
    </aside>
  );
};

/** The right-click menu on a mission row: only the moves its state allows. */
const MissionMenu: React.FC<{
  mission: Mission;
  t: TFunction<"missions">;
  onOpen: () => void;
  onAction: (action: MissionAction) => void;
  onSelectJob: (jobId: string) => void;
}> = ({ mission: m, t, onOpen, onAction, onSelectJob }) => (
  <ContextMenuContent className="min-w-52">
    <ContextMenuItem onSelect={onOpen}>
      <ExternalLink aria-hidden="true" />
      {t("menu.open")}
    </ContextMenuItem>
    {m.currentJob && (
      <ContextMenuItem onSelect={() => onSelectJob(m.currentJob!.jobId)}>
        <Eye aria-hidden="true" />
        {t("live.watch")}
      </ContextMenuItem>
    )}
    {m.state === "AwaitingApproval" && (
      <ContextMenuItem onSelect={() => onAction("approve")}>
        <ThumbsUp aria-hidden="true" />
        {t("actions.approve")}
      </ContextMenuItem>
    )}
    {isLive(m.state) && (
      <ContextMenuItem onSelect={() => onAction("pause")}>
        <Pause aria-hidden="true" />
        {t("actions.pause")}
      </ContextMenuItem>
    )}
    {m.state === "Paused" && (
      <ContextMenuItem onSelect={() => onAction("resume")}>
        <Play aria-hidden="true" />
        {t("actions.resume")}
      </ContextMenuItem>
    )}
    {!isTerminal(m.state) && (
      <>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => onAction("complete")}>
          <CircleCheck aria-hidden="true" />
          {m.currentJob ? t("menu.doneStops") : t("menu.done")}
        </ContextMenuItem>
        <ContextMenuItem
          onSelect={() => onAction("cancel")}
          className="text-destructive focus:text-destructive"
        >
          <Ban aria-hidden="true" />
          {t("menu.cancel")}
        </ContextMenuItem>
      </>
    )}
  </ContextMenuContent>
);

/* --------------------------------------------------------------- detail */

const Banner: React.FC<{
  tone: Tone;
  children: React.ReactNode;
  action?: React.ReactNode;
  testId?: string;
}> = ({ tone, children, action, testId }) => {
  const styles: Record<Tone, string> = {
    ok: "border-primary/25 bg-primary/[0.07] text-success",
    warn: "border-warning/25 bg-warning/[0.07] text-warning",
    bad: "border-destructive/25 bg-destructive/[0.07] text-destructive",
    info: "border-info/25 bg-info/[0.07] text-info",
    mute: "border-border bg-muted text-muted-foreground",
    violet: "border-violet/25 bg-violet/[0.07] text-violet",
  };
  return (
    <div
      data-testid={testId}
      className={cn(
        "flex items-center gap-3 rounded-xl border px-4 py-3 text-[13px] leading-snug",
        styles[tone],
      )}
    >
      <span className="min-w-0 flex-1">{children}</span>
      {action}
    </div>
  );
};

const PhaseStepper: React.FC<{
  state: MissionState;
  pausedFrom?: MissionState;
  t: TFunction<"missions">;
}> = ({ state, pausedFrom, t }) => {
  const effective = state === "Paused" ? pausedFrom : state;
  const current = effective ? (PHASE_OF[effective] ?? -1) : -1;
  const cancelled = state === "Cancelled";
  const complete = state === "Completed";
  return (
    <ol
      className="m-0 grid list-none grid-cols-5 gap-1.5 p-0"
      aria-label={t(`state.${state}`)}
    >
      {PHASES.map((phase, i) => {
        const done = complete || i < current;
        const now = !complete && i === current;
        return (
          <li
            key={phase}
            aria-current={now ? "step" : undefined}
            className="flex flex-col gap-1.5"
          >
            <span
              className={cn(
                "h-1 rounded-full",
                cancelled
                  ? "bg-secondary"
                  : complete
                    ? "bg-primary"
                    : done
                      ? "bg-primary/60"
                      : now
                        ? state === "Paused"
                          ? "bg-warning"
                          : "bg-success shadow-[0_0_10px_rgba(25,224,165,0.5)]"
                        : "bg-secondary",
              )}
            />
            <span
              className={cn(
                "flex items-center gap-1 text-[11.5px]",
                now
                  ? "font-medium text-foreground"
                  : done
                    ? "text-muted-foreground"
                    : "text-muted-foreground/60",
              )}
            >
              {done && !cancelled && (
                <Check
                  size={11}
                  strokeWidth={3}
                  className="text-success"
                  aria-hidden="true"
                />
              )}
              {t(`phase.${phase}`)}
            </span>
          </li>
        );
      })}
    </ol>
  );
};

const MissionDetail: React.FC<{
  mission: Mission;
  config: TendrilConfig | null;
  busy: string | null;
  t: TFunction<"missions">;
  onAction: (action: MissionAction) => void;
  onChangeAgent: (role: MissionRoleName, agent: string) => void;
  onSelectPlan: (planId: string) => void;
  onSelectJob: (jobId: string) => void;
}> = ({
  mission,
  config,
  busy,
  t,
  onAction,
  onChangeAgent,
  onSelectPlan,
  onSelectJob,
}) => {
  const passed = mission.milestones.filter((m) => m.state === "Passed").length;
  const live = mission.milestones.filter((m) => m.state !== "Skipped");
  const current = mission.currentJob;
  const cap = mission.budget.maxCost;
  const overCap = cap != null && mission.cost >= cap;
  const finished = isTerminal(mission.state) || mission.state === "Review";
  const elapsedMs = Math.max(
    0,
    (Date.parse(mission.updated) || 0) - (Date.parse(mission.created) || 0),
  );
  const [tab, setTab] = useState<"crew" | "runs" | "activity">(
    current ? "runs" : "activity",
  );

  const approve = (
    <PrimaryButton
      disabled={busy !== null}
      onClick={() => onAction("approve")}
      data-testid="mission-approve"
    >
      <ThumbsUp size={13} aria-hidden="true" />
      {t("actions.approve")}
    </PrimaryButton>
  );

  return (
    <div className="flex min-w-0 flex-col gap-5" data-testid="mission-detail">
      {/* Hero */}
      <header className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2 font-mono text-[11.5px] text-muted-foreground">
          <span>#{mission.id}</span>
          <Pill
            tone={STATE_TONE[mission.state]}
            dot
            live={isLive(mission.state)}
          >
            {t(`state.${mission.state}`)}
          </Pill>
          <Pill>{mission.project}</Pill>
          {mission.branch && (
            <Pill>
              <GitBranch size={11} aria-hidden="true" />
              {mission.branch}
            </Pill>
          )}
          <span className="font-sans text-xs">
            {t("hero.updated", { ago: sinceIso(mission.updated) })}
          </span>
          <div className="ml-auto flex items-center gap-2 font-sans">
            {isLive(mission.state) && (
              <GhostButton
                disabled={busy !== null}
                onClick={() => onAction("pause")}
              >
                <Pause size={13} aria-hidden="true" />
                {t("actions.pause")}
              </GhostButton>
            )}
            {mission.state === "Paused" && (
              <PrimaryButton
                disabled={busy !== null}
                onClick={() => onAction("resume")}
              >
                <Play size={13} aria-hidden="true" />
                {t("actions.resume")}
              </PrimaryButton>
            )}
            {mission.integrationPlan && (
              <GhostButton
                onClick={() => onSelectPlan(planIdOf(mission.integrationPlan!))}
              >
                <ExternalLink size={13} aria-hidden="true" />
                {t("actions.openPlan")}
              </GhostButton>
            )}
            {!isTerminal(mission.state) && (
              <ContextMenuLikeMore
                t={t}
                disabled={busy !== null}
                onComplete={() => onAction("complete")}
                onCancel={() => onAction("cancel")}
              />
            )}
          </div>
        </div>
        <h1 className="m-0 text-[26px] font-semibold leading-tight tracking-[-0.025em]">
          {mission.title}
        </h1>
        {mission.summary && (
          <p className="m-0 max-w-3xl text-[13.5px] leading-relaxed text-muted-foreground">
            {mission.summary}
          </p>
        )}
        {mission.goal && mission.goal.trim() !== mission.title.trim() && (
          <details className="group max-w-3xl">
            <summary className="flex cursor-pointer list-none items-center gap-1.5 text-[12px] text-muted-foreground hover:text-foreground">
              <ChevronRight
                size={12}
                className="transition-transform group-open:rotate-90"
                aria-hidden="true"
              />
              {t("goal.title")}
            </summary>
            <p className="m-0 mt-2 whitespace-pre-wrap rounded-lg border border-border bg-card px-3 py-2.5 text-[12.5px] leading-relaxed text-foreground/90">
              {mission.goal}
            </p>
          </details>
        )}
      </header>

      <PhaseStepper
        state={mission.state}
        pausedFrom={mission.pausedFrom}
        t={t}
      />

      {mission.state === "AwaitingApproval" && (
        <Banner tone="warn" action={approve}>
          {t("banner.approve")}
        </Banner>
      )}
      {mission.rateLimit && (
        <Banner tone="info" testId="mission-rate-limited">
          {t("banner.rateLimited", {
            reason: mission.rateLimit.reason,
            step: mission.rateLimit.milestone
              ? `${mission.rateLimit.step} ${mission.rateLimit.milestone}`
              : mission.rateLimit.step,
            time: new Date(mission.rateLimit.until).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            }),
          })}
        </Banner>
      )}
      {mission.state === "Paused" && mission.pauseReason && (
        <Banner tone="warn" testId="mission-paused">
          {t("banner.paused", { reason: mission.pauseReason })}
        </Banner>
      )}
      {mission.state === "Planning" && (
        <Banner tone="info">
          <span className="inline-flex items-center gap-2">
            <Spinner size="sm" />
            {t("banner.planning")}
          </span>
        </Banner>
      )}
      {mission.state === "Review" && (
        <Banner tone="ok">{t("banner.review")}</Banner>
      )}

      {/* Stats */}
      <div
        data-testid="mission-stats"
        className="grid shrink-0 grid-cols-2 overflow-hidden rounded-xl border border-border bg-card lg:grid-cols-4"
      >
        <Stat label={t("stats.progress")}>
          <span className="text-[22px] font-semibold leading-none tracking-[-0.02em]">
            {passed}
            <span className="text-sm font-normal text-muted-foreground">
              /{live.length}
            </span>
          </span>
          <Segments milestones={mission.milestones} />
        </Stat>
        <Stat label={t("stats.cost")} divider>
          <span
            className={cn(
              "text-[22px] font-semibold leading-none tracking-[-0.02em]",
              overCap && "text-destructive",
            )}
          >
            {money(mission.cost)}
          </span>
          {cap != null ? (
            <span className="flex items-center gap-2">
              <span className="block h-1 flex-1 overflow-hidden rounded-full bg-secondary">
                <span
                  className={cn(
                    "block h-full rounded-full",
                    overCap ? "bg-destructive" : "bg-violet",
                  )}
                  style={{
                    width: `${Math.min(100, (mission.cost / cap) * 100)}%`,
                  }}
                />
              </span>
              <span className="font-mono text-[10.5px] text-muted-foreground">
                {money(cap)}
              </span>
            </span>
          ) : (
            <span className="text-[11px] text-muted-foreground">
              {t("stats.noCap")}
            </span>
          )}
        </Stat>
        {current ? (
          <Stat
            label={t("stats.step")}
            divider
            onClick={() => onSelectJob(current.jobId)}
          >
            <span className="flex items-center gap-2 text-[15px] font-semibold leading-tight">
              <span
                className="size-1.5 animate-pulse rounded-full bg-success"
                aria-hidden="true"
              />
              {t(`step.${current.step}`)}
              {current.milestone ? ` ${current.milestone}` : ""}
            </span>
            <span className="inline-flex items-center gap-1 text-[11px] text-success">
              <Eye size={11} aria-hidden="true" />
              {t("live.watch")}
            </span>
          </Stat>
        ) : (
          <Stat label={finished ? t("stats.took") : t("stats.running")} divider>
            <span className="text-[22px] font-semibold leading-none tracking-[-0.02em]">
              {elapsedMs > 0 ? formatAge(elapsedMs) : "—"}
            </span>
            <span className="text-[11px] text-muted-foreground">
              {t("stats.since", { ago: sinceIso(mission.created) })}
            </span>
          </Stat>
        )}
        <Stat label={t("stats.runs")} divider>
          <span className="text-[22px] font-semibold leading-none tracking-[-0.02em]">
            {mission.jobs?.length ?? 0}
          </span>
          <span className="text-[11px] text-muted-foreground">
            {mission.replans > 0
              ? t("stats.replans", {
                  count: mission.replans,
                  max: mission.budget.maxReplans,
                })
              : t("stats.attemptsShort", { max: mission.budget.maxAttempts })}
          </span>
        </Stat>
      </div>

      <div className="grid min-w-0 items-start gap-5 xl:grid-cols-[minmax(0,1fr)_340px]">
        <section
          className="flex min-w-0 flex-col gap-2"
          data-testid="mission-milestones"
        >
          <div className="flex items-center gap-2">
            <h2 className="m-0 text-[13px] font-semibold">
              {t("milestones.title")}
            </h2>
            <span className="font-mono text-[11px] text-muted-foreground">
              {t("list.progress", { passed, total: live.length })}
            </span>
          </div>
          {mission.milestones.length === 0 ? (
            <p className="m-0 rounded-xl border border-dashed border-border px-4 py-8 text-center text-[12.5px] text-muted-foreground">
              {t("milestones.none")}
            </p>
          ) : (
            <ol className="m-0 flex list-none flex-col gap-2 p-0">
              {mission.milestones.map((m) => (
                <MilestoneRow
                  key={m.id}
                  milestone={m}
                  all={mission.milestones}
                  reviewing={mission.state === "AwaitingApproval"}
                  maxAttempts={mission.budget.maxAttempts}
                  activeAgent={
                    current?.milestone === m.id ? current.agent : undefined
                  }
                  onWatch={
                    current?.milestone === m.id
                      ? () => onSelectJob(current.jobId)
                      : undefined
                  }
                  t={t}
                  onSelectPlan={onSelectPlan}
                />
              ))}
            </ol>
          )}
        </section>

        <div className="flex min-w-0 flex-col gap-3 xl:sticky xl:top-0">
          {current && (
            <LiveAgentCard
              mission={mission}
              job={current}
              t={t}
              onWatch={() => onSelectJob(current.jobId)}
            />
          )}
          <Card
            testId="mission-side"
            title={
              <span
                role="tablist"
                aria-label={t("side.label")}
                className="flex gap-4"
              >
                {(["crew", "runs", "activity"] as const).map((k) => (
                  <button
                    key={k}
                    type="button"
                    role="tab"
                    aria-selected={tab === k}
                    onClick={() => setTab(k)}
                    className={cn(
                      "-mb-[13px] flex h-11 items-center gap-1.5 border-b-2 text-[13px] transition-colors",
                      tab === k
                        ? "border-primary font-semibold text-foreground"
                        : "border-transparent font-normal text-muted-foreground hover:text-foreground",
                    )}
                  >
                    {t(`side.${k}`)}
                    {k === "runs" && (mission.jobs?.length ?? 0) > 0 && (
                      <span className="font-mono text-[10.5px] text-muted-foreground">
                        {mission.jobs!.length}
                      </span>
                    )}
                  </button>
                ))}
              </span>
            }
          >
            {tab === "crew" && (
              <div data-testid="mission-agents">
                {ROLES.map((role) => (
                  <div
                    key={role}
                    className="flex items-center gap-3 border-b border-border/70 px-4 py-2 last:border-b-0"
                    title={t(`agents.describe.${role}`)}
                  >
                    <span className="flex-1 text-[12.5px]">
                      {t(`agents.roles.${role}`)}
                    </span>
                    <AgentPicker
                      label={t(`agents.roles.${role}`)}
                      value={mission.agents?.[role]?.agent ?? ""}
                      defaultLabel={t("agents.default")}
                      options={visibleAgents(config, [
                        mission.agents?.[role]?.agent,
                      ]).map((a) => ({
                        value: a.id,
                        label: a.label,
                      }))}
                      side="bottom"
                      disabled={
                        isTerminal(mission.state) || busy === `agent-${role}`
                      }
                      onChange={(agent) => onChangeAgent(role, agent)}
                    />
                  </div>
                ))}
                <p className="m-0 px-4 py-2.5 text-[11px] leading-snug text-muted-foreground">
                  {t("agents.hint")}
                </p>
              </div>
            )}
            {tab === "runs" && (
              <ul
                data-testid="mission-runs"
                className="m-0 max-h-[440px] list-none overflow-y-auto p-0"
              >
                {(mission.jobs?.length ?? 0) === 0 && (
                  <li className="px-4 py-6 text-center text-[12px] text-muted-foreground">
                    {t("runs.none")}
                  </li>
                )}
                {[...(mission.jobs ?? [])].reverse().map((job) => {
                  const running = job.jobId === current?.jobId;
                  return (
                    <li key={job.jobId}>
                      <button
                        type="button"
                        onClick={() => onSelectJob(job.jobId)}
                        className={cn(
                          "group flex h-10 w-full items-center gap-2.5 border-b border-border/70 px-4 text-left text-[12.5px] transition-colors hover:bg-muted/50",
                          running && "bg-primary/[0.05]",
                        )}
                      >
                        <span
                          className={cn(
                            "size-1.5 shrink-0 rounded-full",
                            running
                              ? "animate-pulse bg-success"
                              : "bg-primary/50",
                          )}
                          aria-hidden="true"
                        />
                        <span className="font-medium">
                          {t(`step.${job.step}`)}
                        </span>
                        {job.milestone && (
                          <span className="rounded bg-secondary px-1 font-mono text-[10.5px] text-muted-foreground">
                            {job.milestone}
                          </span>
                        )}
                        <span className="min-w-0 flex-1 truncate text-right font-mono text-[10.5px] text-muted-foreground">
                          {job.agent?.split(" · ")[0]}
                        </span>
                        <Eye
                          size={13}
                          className="shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100"
                          aria-hidden="true"
                        />
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            {tab === "activity" && (
              <ul
                data-testid="mission-log"
                className="relative m-0 max-h-[440px] list-none overflow-y-auto px-4 py-2"
              >
                {(mission.log ?? []).length === 0 && (
                  <li className="py-6 text-center text-[12px] text-muted-foreground">
                    {t("runs.none")}
                  </li>
                )}
                {[...(mission.log ?? [])]
                  .reverse()
                  .slice(0, 80)
                  .map((entry, i) => (
                    <LogEntry key={`${entry.at}-${i}`} entry={entry} />
                  ))}
              </ul>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
};

/** One orchestrator log line, clamped to two lines until clicked. */
const LogEntry: React.FC<{ entry: NonNullable<Mission["log"]>[number] }> = ({
  entry,
}) => {
  const [open, setOpen] = useState(false);
  return (
    <li className="relative flex gap-3 py-1.5">
      <span
        className="mt-[6px] size-1.5 shrink-0 rounded-full bg-border"
        aria-hidden="true"
      />
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "min-w-0 flex-1 text-left text-[12px] leading-snug text-foreground/90",
          !open && "line-clamp-2",
        )}
      >
        {entry.milestone && (
          <span className="mr-1.5 font-mono text-[11px] text-success">
            {entry.milestone}
          </span>
        )}
        {entry.message}
      </button>
      <span className="shrink-0 pt-px font-mono text-[10.5px] text-muted-foreground">
        {new Date(entry.at).toLocaleTimeString([], {
          hour: "2-digit",
          minute: "2-digit",
        })}
      </span>
    </li>
  );
};

/** Mark as done and Cancel, tucked behind one button: final actions should take a deliberate click. */
const ContextMenuLikeMore: React.FC<{
  t: TFunction<"missions">;
  disabled: boolean;
  onComplete: () => void;
  onCancel: () => void;
}> = ({ t, disabled, onComplete, onCancel }) => (
  <DropdownMenu>
    <DropdownMenuTrigger asChild>
      <GhostButton
        disabled={disabled}
        aria-label={t("menu.more")}
        className="px-2"
      >
        <MoreHorizontal size={15} aria-hidden="true" />
      </GhostButton>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" className="min-w-48">
      <DropdownMenuItem onSelect={onComplete} data-testid="mission-complete">
        <CircleCheck aria-hidden="true" />
        {t("menu.done")}
      </DropdownMenuItem>
      <DropdownMenuItem
        onSelect={onCancel}
        className="text-destructive focus:text-destructive"
      >
        <Ban aria-hidden="true" />
        {t("menu.cancel")}
      </DropdownMenuItem>
    </DropdownMenuContent>
  </DropdownMenu>
);

const Stat: React.FC<{
  label: string;
  divider?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}> = ({ label, divider, onClick, children }) => {
  const cls = cn(
    "flex min-w-0 flex-col gap-2 px-4 py-3.5 text-left",
    divider && "border-l border-border/70",
    onClick && "transition-colors hover:bg-muted/60",
  );
  const body = (
    <>
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </>
  );
  return onClick ? (
    <button type="button" className={cls} onClick={onClick}>
      {body}
    </button>
  ) : (
    <div className={cls}>{body}</div>
  );
};

/* ------------------------------------------------------------ milestones */

const MilestoneIcon: React.FC<{ state: MilestoneState }> = ({ state }) => {
  const base =
    "relative z-10 flex size-6 shrink-0 items-center justify-center rounded-full border";
  switch (state) {
    case "Passed":
      return (
        <span
          className={cn(
            base,
            "border-primary bg-primary text-primary-foreground",
          )}
        >
          <Check size={13} strokeWidth={3} aria-hidden="true" />
        </span>
      );
    case "Executing":
      return (
        <span
          className={cn(
            base,
            "border-primary bg-background text-success shadow-[0_0_0_4px_rgba(0,204,146,0.12)]",
          )}
        >
          <span className="size-2 animate-pulse rounded-full bg-current" />
        </span>
      );
    case "Judging":
      return (
        <span
          className={cn(
            base,
            "border-violet bg-background text-violet shadow-[0_0_0_4px_color-mix(in_srgb,var(--violet)_14%,transparent)]",
          )}
        >
          <span className="size-2 animate-pulse rounded-full bg-current" />
        </span>
      );
    case "Skipped":
      return (
        <span
          className={cn(
            base,
            "border-border bg-background text-muted-foreground",
          )}
        >
          <Ban size={11} aria-hidden="true" />
        </span>
      );
    default:
      return (
        <span
          className={cn(
            base,
            "border-input bg-background text-muted-foreground",
          )}
        >
          <CircleDashed size={12} aria-hidden="true" />
        </span>
      );
  }
};

const AttemptDots: React.FC<{
  attempts: number;
  max: number;
  label: string;
}> = ({ attempts, max, label }) => (
  <span
    className="inline-flex items-center gap-[3px]"
    title={label}
    aria-label={label}
  >
    {Array.from({ length: Math.max(max, attempts) }, (_, i) => (
      <span
        key={i}
        className={cn(
          "size-[5px] rounded-full",
          i < attempts
            ? attempts > 1
              ? "bg-warning"
              : "bg-success"
            : "bg-secondary",
        )}
      />
    ))}
  </span>
);

/** Renders `inline code` spans in a line of prose. */
const withCode = (text: string): React.ReactNode =>
  text.split(/(`[^`]+`)/g).map((part, i) =>
    part.startsWith("`") && part.endsWith("`") && part.length > 2 ? (
      <code
        key={i}
        className="rounded bg-secondary px-1 py-px font-mono text-[11px] text-foreground"
      >
        {part.slice(1, -1)}
      </code>
    ) : (
      <React.Fragment key={i}>{part}</React.Fragment>
    ),
  );

const MilestoneRow: React.FC<{
  milestone: Milestone;
  /** Every milestone, for naming which one provides what this one consumes. */
  all: Milestone[];
  /** The plan is up for approval: every row opens, so the whole plan can be read before approving. */
  reviewing?: boolean;
  maxAttempts: number;
  activeAgent?: string;
  onWatch?: () => void;
  t: TFunction<"missions">;
  onSelectPlan: (planId: string) => void;
}> = ({ milestone: m, all, reviewing = false, maxAttempts, activeAgent, onWatch, t, onSelectPlan }) => {
  const active = m.state === "Executing" || m.state === "Judging";
  // Settled milestones fold to one line; the one in motion, one the judge sent back, and every one
  // while the plan waits for approval stay open.
  const [open, setOpen] = useState(
    reviewing || active || (!!m.feedback && m.state !== "Passed"),
  );
  const tasks = m.tasks ?? [];
  const tasksDone = tasks.filter((task) => task.done).length;
  const provider = (name: string) =>
    all.find((other) => other.provides?.some((p) => p.name.toLowerCase() === name.toLowerCase()))?.id;
  return (
    <li
      data-testid={`milestone-${m.id}`}
      className={cn(
        "overflow-hidden rounded-xl border bg-card",
        active
          ? "border-primary/30 shadow-[0_14px_36px_-24px_rgba(0,204,146,0.5)]"
          : "border-border",
        m.state === "Skipped" && "opacity-60",
      )}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
      >
        <MilestoneIcon state={m.state} />
        <span className="font-mono text-[11px] text-muted-foreground">
          {m.id}
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span
            className={cn(
              "truncate text-[13.5px] font-medium",
              m.state === "Skipped" && "line-through",
            )}
          >
            {m.title}
          </span>
          {!open && m.summary && m.state === "Passed" && (
            <span className="truncate text-[12px] text-muted-foreground">
              {m.summary}
            </span>
          )}
        </span>
        {tasks.length > 0 && (
          <span className="font-mono text-[11px] text-muted-foreground">
            {t("milestones.taskProgress", { done: tasksDone, total: tasks.length })}
          </span>
        )}
        {m.attempts > 0 && (
          <AttemptDots
            attempts={m.attempts}
            max={maxAttempts}
            label={t("milestones.attempt", {
              attempt: m.attempts,
              max: maxAttempts,
            })}
          />
        )}
        {m.cost > 0 && (
          <span className="font-mono text-[11px] text-muted-foreground">
            {money(m.cost)}
          </span>
        )}
        <Pill tone={MILESTONE_TONE[m.state]} dot={active} live={active}>
          {t(`milestoneState.${m.state}`)}
        </Pill>
        <ChevronRight
          size={14}
          className={cn(
            "shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90",
          )}
          aria-hidden="true"
        />
      </button>

      {open && (
        <div className="flex flex-col gap-3 border-t border-border/70 px-4 pb-4 pl-[52px] pt-3">
          {activeAgent && (
            <button
              type="button"
              onClick={onWatch}
              className="inline-flex items-center gap-1.5 self-start rounded-md border border-primary/25 bg-primary/[0.06] px-2 py-1 font-mono text-[11px] text-success hover:bg-primary/[0.12]"
            >
              <Eye size={12} aria-hidden="true" />
              {activeAgent}
            </button>
          )}
          {m.objective && (
            <p className="m-0 text-[12.5px] leading-relaxed text-muted-foreground">
              {withCode(m.objective)}
            </p>
          )}
          {m.acceptance.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
                {t("milestones.criteria")}
              </span>
              <ul className="m-0 flex list-none flex-col gap-1 p-0">
                {m.acceptance.map((a) => (
                  <li
                    key={a}
                    className="flex items-start gap-2 text-[12.5px] leading-snug text-foreground/90"
                  >
                    <span
                      className={cn(
                        "mt-[3px] shrink-0",
                        m.state === "Passed"
                          ? "text-success"
                          : "text-muted-foreground",
                      )}
                    >
                      {m.state === "Passed" ? (
                        <Check size={12} strokeWidth={3} aria-hidden="true" />
                      ) : (
                        <CircleDashed size={12} aria-hidden="true" />
                      )}
                    </span>
                    <span>{withCode(a)}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {tasks.length > 0 && (
            <div className="flex flex-col gap-1.5" data-testid={`milestone-${m.id}-tasks`}>
              <span className="font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
                {t("milestones.tasks")}
              </span>
              <ol className="m-0 flex list-none flex-col gap-1 p-0">
                {tasks.map((task) => (
                  <li key={task.id} className="flex items-start gap-2 text-[12.5px] leading-snug">
                    <span className={cn("mt-[3px] shrink-0", task.done ? "text-success" : "text-muted-foreground")}>
                      {task.done ? (
                        <Check size={12} strokeWidth={3} aria-hidden="true" />
                      ) : (
                        <CircleDashed size={12} aria-hidden="true" />
                      )}
                    </span>
                    <span className="font-mono text-[11px] text-muted-foreground">{task.id}</span>
                    <span className="flex min-w-0 flex-col">
                      <span className={cn("text-foreground/90", task.done && "text-muted-foreground line-through")}>
                        {withCode(task.title)}
                      </span>
                      {task.doneWhen && (
                        <span className="text-[11.5px] text-muted-foreground">
                          {t("milestones.doneWhen", { doneWhen: task.doneWhen })}
                        </span>
                      )}
                    </span>
                  </li>
                ))}
              </ol>
            </div>
          )}
          {((m.consumes?.length ?? 0) > 0 || (m.provides?.length ?? 0) > 0) && (
            <div className="flex flex-col gap-2" data-testid={`milestone-${m.id}-contract`}>
              <span className="font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
                {t("milestones.contract")}
              </span>
              {(m.consumes?.length ?? 0) > 0 && (
                <div className="flex flex-wrap items-center gap-1.5 text-[12px]">
                  <span className="text-muted-foreground">{t("milestones.consumes")}</span>
                  {m.consumes!.map((name) => (
                    <span key={name} className="rounded-md border border-border bg-muted/40 px-1.5 py-0.5 font-mono text-[11px]">
                      {name}
                      {provider(name) && (
                        <span className="ml-1 text-muted-foreground">{t("milestones.from", { id: provider(name) })}</span>
                      )}
                    </span>
                  ))}
                </div>
              )}
              {(m.provides?.length ?? 0) > 0 && (
                <div className="flex flex-col gap-1">
                  <span className="text-[12px] text-muted-foreground">{t("milestones.provides")}</span>
                  <ul className="m-0 flex list-none flex-col gap-1 p-0">
                    {m.provides!.map((item) => (
                      <li key={item.name} className="rounded-md border border-primary/20 bg-primary/[0.04] px-2 py-1.5">
                        <div className="flex flex-wrap items-baseline gap-x-2">
                          <span className="font-mono text-[12px] font-medium text-foreground">{item.name}</span>
                          {item.kind && <span className="text-[11px] text-muted-foreground">{item.kind}</span>}
                          {item.location && (
                            <span className="font-mono text-[11px] text-muted-foreground">{item.location}</span>
                          )}
                        </div>
                        {item.signature && (
                          <code className="mt-0.5 block whitespace-pre-wrap break-words font-mono text-[11px] text-foreground/80">
                            {item.signature}
                          </code>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          )}
          {m.spec && (
            <details className="group rounded-lg border border-border/70" open={reviewing && all.length <= 3}>
              <summary className="cursor-pointer select-none px-3 py-2 text-[12px] font-medium text-muted-foreground hover:text-foreground">
                {t("milestones.spec")}
              </summary>
              <div className="border-t border-border/70 px-3 py-2 text-[12.5px]">
                <PlanMarkdown id={`milestone-spec-${m.id}`} content={m.spec} article flow />
              </div>
            </details>
          )}
          {m.feedback && m.state !== "Passed" && (
            <p className="m-0 rounded-lg border border-warning/20 bg-warning/[0.07] px-3 py-2 text-[12px] leading-snug text-warning">
              {t("milestones.feedback", { feedback: m.feedback })}
            </p>
          )}
          {m.summary && m.state === "Passed" && (
            <p className="m-0 rounded-lg border border-primary/20 bg-primary/[0.05] px-3 py-2 text-[12.5px] leading-snug text-foreground/90">
              <span className="mr-1.5 font-medium text-success">
                {t("milestones.result")}
              </span>
              {withCode(m.summary)}
            </p>
          )}
          <div className="flex items-center gap-3 font-mono text-[11px] text-muted-foreground">
            {(m.commits?.length ?? 0) > 0 && (
              <span>
                {t("milestones.commits", { count: m.commits!.length })}
              </span>
            )}
            {m.plan && (
              <button
                type="button"
                onClick={() => onSelectPlan(planIdOf(m.plan!))}
                className="inline-flex items-center gap-1 hover:text-success"
              >
                {t("milestones.openPlan", { id: planIdOf(m.plan) })}
                <ExternalLink size={11} aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
      )}
    </li>
  );
};

/* ------------------------------------------------------------- live agent */

const LiveAgentCard: React.FC<{
  mission: Mission;
  job: NonNullable<Mission["currentJob"]>;
  t: TFunction<"missions">;
  onWatch: () => void;
}> = ({ mission, job, t, onWatch }) => {
  const milestone = mission.milestones.find((m) => m.id === job.milestone);
  const [agent, ...rest] = (job.agent ?? "").split(" · ");
  return (
    <section
      data-testid="mission-live-agent"
      className="flex flex-col gap-3 rounded-xl border border-primary/25 bg-primary/[0.05] p-4 shadow-[0_18px_40px_-24px_rgba(0,204,146,0.45)]"
    >
      <div className="flex items-center gap-2">
        <span
          className="size-2 animate-pulse rounded-full bg-success shadow-[0_0_0_4px_rgba(0,204,146,0.18)]"
          aria-hidden="true"
        />
        <span className="text-[12px] font-medium text-success">
          {t("live.title")}
        </span>
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">
          {job.jobId}
        </span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[14px] font-semibold">
          {t(`step.${job.step}`)}
          {milestone ? ` · ${milestone.title}` : ""}
        </span>
        {agent && (
          <span className="font-mono text-[11.5px] text-muted-foreground">
            {agent}
            {rest.length > 0 && (
              <span className="opacity-70"> · {rest.join(" · ")}</span>
            )}
          </span>
        )}
      </div>
      <PrimaryButton
        onClick={onWatch}
        className="justify-center"
        data-testid="mission-watch"
      >
        <Eye size={14} aria-hidden="true" />
        {t("live.watch")}
      </PrimaryButton>
    </section>
  );
};

/* --------------------------------------------------------------- overview */

const MissionsOverview: React.FC<{
  missions: Mission[];
  t: TFunction<"missions">;
  onOpen: (id: string) => void;
  onSelectJob: (jobId: string) => void;
}> = ({ missions, t, onOpen, onSelectJob }) => {
  const active = missions.filter((m) => !isTerminal(m.state));
  const finished = missions.filter((m) => isTerminal(m.state)).slice(0, 6);
  const projects = new Set(active.map((m) => m.project)).size;
  const running = active.filter((m) => m.currentJob).length;
  const spend = active.reduce((s, m) => s + m.cost, 0);

  const card = (m: Mission) => {
    const passed = m.milestones.filter((ms) => ms.state === "Passed").length;
    const total = m.milestones.filter((ms) => ms.state !== "Skipped").length;
    return (
      <div
        key={m.id}
        className={cn(
          "flex flex-col gap-3 rounded-xl border bg-card p-4 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]",
          m.currentJob ? "border-primary/25" : "border-border",
        )}
      >
        <button
          type="button"
          onClick={() => onOpen(m.id)}
          className="flex flex-col gap-2 text-left"
        >
          <span className="flex items-center gap-2">
            <span className="font-mono text-[10.5px] text-muted-foreground">
              #{m.id}
            </span>
            <Pill className="h-[18px] text-[10.5px]">{m.project}</Pill>
            <Pill
              tone={STATE_TONE[m.state]}
              dot
              live={isLive(m.state)}
              className="ml-auto h-[18px] text-[10.5px]"
            >
              {t(`state.${m.state}`)}
            </Pill>
          </span>
          <span className="text-[14px] font-semibold leading-snug hover:text-success">
            {m.title}
          </span>
        </button>
        <PhaseStepper state={m.state} pausedFrom={m.pausedFrom} t={t} />
        <div className="flex items-center gap-3">
          <span className="flex-1">
            <Segments milestones={m.milestones} />
          </span>
          <span className="font-mono text-[11px] text-muted-foreground">
            {passed}/{total} · {money(m.cost)}
          </span>
        </div>
        {m.currentJob ? (
          <button
            type="button"
            onClick={() => onSelectJob(m.currentJob!.jobId)}
            className="flex items-center gap-2.5 rounded-lg border border-primary/20 bg-primary/[0.05] px-3 py-2 text-left transition-colors hover:bg-primary/[0.1]"
          >
            <span
              className="size-1.5 shrink-0 animate-pulse rounded-full bg-success"
              aria-hidden="true"
            />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-[12.5px]">
                {t(`step.${m.currentJob.step}`)}
                {m.currentJob.milestone ? ` ${m.currentJob.milestone}` : ""}
              </span>
              {m.currentJob.agent && (
                <span className="truncate font-mono text-[10.5px] text-muted-foreground">
                  {m.currentJob.agent}
                </span>
              )}
            </span>
            <span className="inline-flex shrink-0 items-center gap-1 text-[12px] font-medium text-success">
              <Eye size={13} aria-hidden="true" />
              {t("live.watch")}
            </span>
          </button>
        ) : m.state === "AwaitingApproval" ? (
          <button
            type="button"
            onClick={() => onOpen(m.id)}
            className="rounded-lg border border-warning/25 bg-warning/[0.07] px-3 py-2 text-left text-[12.5px] text-warning hover:bg-warning/[0.12]"
          >
            {t("overview.needsApproval")}
          </button>
        ) : m.state === "Paused" ? (
          <span className="rounded-lg border border-warning/25 bg-warning/[0.07] px-3 py-2 text-[12.5px] text-warning">
            {m.pauseReason
              ? t("banner.paused", { reason: m.pauseReason })
              : t("state.Paused")}
          </span>
        ) : null}
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-5" data-testid="missions-overview">
      <header className="flex flex-col gap-1.5">
        <h1 className="m-0 text-[26px] font-semibold tracking-[-0.025em]">
          {t("overview.title")}
        </h1>
        <p className="m-0 text-[13px] text-muted-foreground">
          {t("overview.summary", {
            missions: active.length,
            projects,
            running,
            spend: money(spend),
          })}
        </p>
      </header>
      {active.length === 0 ? (
        <p className="m-0 rounded-xl border border-border bg-card px-4 py-8 text-center text-[13px] text-muted-foreground">
          {t("overview.none")}
        </p>
      ) : (
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2 2xl:grid-cols-3">
          {active.map(card)}
        </div>
      )}
      {finished.length > 0 && (
        <section className="flex flex-col gap-2">
          <h2 className="m-0 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
            {t("overview.finished")}
          </h2>
          <div className="overflow-hidden rounded-xl border border-border bg-card">
            {finished.map((m) => (
              <button
                key={m.id}
                type="button"
                onClick={() => onOpen(m.id)}
                className="flex h-10 w-full items-center gap-3 border-b border-border/70 px-4 text-left text-[13px] last:border-b-0 hover:bg-muted/50"
              >
                <span className="font-mono text-[11px] text-muted-foreground">
                  #{m.id}
                </span>
                <span className="min-w-0 flex-1 truncate">{m.title}</span>
                <span className="font-mono text-[11px] text-muted-foreground">
                  {m.project}
                </span>
                <Pill tone={STATE_TONE[m.state]}>{t(`state.${m.state}`)}</Pill>
                <span className="w-16 text-right font-mono text-[11px] text-muted-foreground">
                  {money(m.cost)}
                </span>
              </button>
            ))}
          </div>
        </section>
      )}
    </div>
  );
};
