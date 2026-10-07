import React from "react";
import { ArrowLeft, Box, Brain, Network, SquareTerminal } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { Job, Mission, ProjectSummary } from "../../types/api";
import type { ProjectDocker, ProjectMemoryEntry } from "../../types/projectAssets";
import { bridge } from "../../api/bridge";
import { usePortForwards } from "../../api/portForwards";
import { chatStore } from "../../state/chatStore";
import { recordRecentProject } from "../../state/recentProjects";
import { useManagerActivity } from "../../state/managerActivity";
import { subscribeProjectIntent, takeProjectIntent, type ProjectTab } from "../../state/projectIntent";
import { isLiveMission, projectStatus } from "../../utils/projectStatus";
import { formatAge } from "../../utils/commandCenter";
import type { ChatMessage } from "../../types/chat";
import { Card, Dot, GhostButton, Label, Pill, PrimaryButton, Seg, type Tone } from "../../components/page/kit";
import { ChatView } from "../ChatView";
import { useMissions } from "./ProjectsHomeView";
import { MissionGraph, MissionRun } from "./MissionRun";
import { ModelsPanel } from "./ModelsPanel";
import { ArtifactsPanel } from "./ArtifactsPanel";

interface Props {
  project: ProjectSummary;
  jobs: Job[];
  onBack: () => void;
  onOpenMission: (missionId: string) => void;
  onOpenJob: (jobId: string) => void;
  onOpenPlan: (planId: string) => void;
  /** Opens the Git page on this project's repositories. */
  onOpenGit?: () => void;
}

type Status = ReturnType<typeof projectStatus>;
type Tab = ProjectTab;
type Side = "chat" | "panels";

const MISSION_TONE: Record<string, Tone> = {
  Planning: "info",
  AwaitingApproval: "warn",
  Running: "ok",
  Validating: "info",
  Review: "violet",
  Completed: "mute",
  Paused: "warn",
  Cancelled: "mute",
};

const useAsync = <T,>(load: () => Promise<T>, deps: React.DependencyList, intervalMs = 20_000) => {
  const [value, setValue] = React.useState<T | null>(null);
  React.useEffect(() => {
    let live = true;
    const run = () =>
      load()
        .then((v) => live && setValue(v))
        .catch(() => undefined);
    void run();
    const timer = window.setInterval(run, intervalMs);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return value;
};

/**
 * The project page: the manager chat on the left half, always mounted so a running conversation is
 * never torn down, and the project's panels on the right half, in tabs.
 */
export const ProjectView: React.FC<Props> = ({ project, jobs, onBack, onOpenMission, onOpenJob, onOpenPlan, onOpenGit }) => {
  const [tab, setTab] = React.useState<Tab>("overview");
  const [side, setSide] = React.useState<Side>("chat");
  const [runId, setRunId] = React.useState<string | null>(null);
  const [draft, setDraft] = React.useState<{ text: string; token: number } | undefined>();
  const missions = useMissions(10_000);
  const activity = useManagerActivity();
  const managerWorking = activity[project.name]?.working ?? false;
  const status = React.useMemo(
    () => projectStatus(project.name, missions, jobs, managerWorking),
    [project.name, missions, jobs, managerWorking],
  );

  React.useEffect(() => recordRecentProject(project.name), [project.name]);

  // A palette jump can arrive before this page mounts or while it is open.
  React.useEffect(() => {
    const apply = () => {
      const intent = takeProjectIntent();
      if (!intent) return;
      if (intent.draft) {
        setSide("chat");
        setDraft({ text: intent.draft, token: Date.now() });
        return;
      }
      setSide("panels");
      if (intent.tab) setTab(intent.tab);
      setRunId(intent.missionId ?? null);
    };
    apply();
    return subscribeProjectIntent(apply);
  }, []);

  const openRun = (id: string) => {
    setTab("missions");
    setSide("panels");
    setRunId(id);
  };
  const pickTab = (next: Tab) => {
    setTab(next);
    if (next !== "missions") setRunId(null);
  };

  const tabs: { value: Tab; label: string; count?: number }[] = [
    { value: "overview", label: "Overview" },
    { value: "missions", label: "Missions", count: status.live.length },
    { value: "tasks", label: "Tasks", count: status.taskCount },
    { value: "artifacts", label: "Artifacts" },
    { value: "models", label: "Models" },
    { value: "runtime", label: "Runtime" },
    { value: "memory", label: "Memory" },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="project-view">
      <header className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-border/70 px-3 py-2.5 lg:flex-nowrap lg:px-5 lg:py-3">
        <GhostButton size="sm" onClick={onBack} aria-label="Back to projects">
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          <span className="hidden sm:inline">Projects</span>
        </GhostButton>
        <h1 className="m-0 min-w-0 truncate text-[18px] font-semibold tracking-[-0.02em] text-foreground">{project.name}</h1>
        <Pill tone={status.busy ? "ok" : "mute"} dot live={status.busy}>
          {status.busy ? "Working" : "Idle"}
        </Pill>
        {onOpenGit && (
          <GhostButton size="sm" className="ml-auto shrink-0" onClick={onOpenGit} title="Branches, commits and uncommitted work in this project's repositories">
            Git
          </GhostButton>
        )}
        {/* Below `lg` there is room for one half at a time. */}
        <Seg<Side>
          className="order-last w-full lg:hidden [&>*]:flex-1"
          label="Show"
          value={side}
          onChange={setSide}
          options={[
            { value: "chat", label: "Chat" },
            { value: "panels", label: "Project" },
          ]}
        />
      </header>

      <div className="flex min-h-0 flex-1">
        <section
          className={cn(
            "min-w-0 flex-col border-border/70 lg:flex lg:w-1/2 lg:border-r",
            side === "chat" ? "flex flex-1" : "hidden",
          )}
          aria-label="Manager chat"
        >
          <ManagerChat project={project} onOpenPlan={onOpenPlan} draft={draft} />
        </section>

        <section
          className={cn("min-w-0 flex-col lg:flex lg:w-1/2", side === "panels" ? "flex flex-1" : "hidden")}
          aria-label="Project panels"
        >
          <SummaryStrip status={status} />
          <nav className="flex shrink-0 gap-1 overflow-x-auto border-b border-border/70 px-3" aria-label="Project panels">
            {tabs.map((t) => (
              <button
                key={t.value}
                type="button"
                onClick={() => pickTab(t.value)}
                aria-current={tab === t.value ? "page" : undefined}
                className={cn(
                  "relative inline-flex h-10 shrink-0 items-center gap-1.5 px-3 text-[12.5px] transition-colors",
                  tab === t.value ? "text-foreground" : "text-muted-foreground hover:text-foreground",
                )}
              >
                {t.label}
                {t.count ? (
                  <span className="rounded-full bg-secondary px-1.5 font-mono text-[10.5px] text-muted-foreground">{t.count}</span>
                ) : null}
                {tab === t.value && <span className="absolute inset-x-3 bottom-0 h-0.5 rounded-full bg-primary" />}
              </button>
            ))}
          </nav>
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            {tab === "overview" && (
              <OverviewPanel
                status={status}
                onOpenMission={openRun}
                onOpenFull={onOpenMission}
                onAskManager={() => setSide("chat")}
              />
            )}
            {tab === "missions" &&
              (runId ? (
                <MissionRun
                  missionId={runId}
                  onBack={() => setRunId(null)}
                  onOpenJob={onOpenJob}
                  onOpenFull={onOpenMission}
                />
              ) : (
                <MissionsPanel project={project} missions={missions} onOpen={openRun} />
              ))}
            {tab === "tasks" && <TasksPanel status={status} onOpenJob={onOpenJob} />}
            {tab === "artifacts" && (
              <ArtifactsPanel
                project={project.name}
                busy={status.busy}
                onOpenMission={openRun}
              />
            )}
            {tab === "models" && <ModelsPanel project={project} missions={missions} />}
            {tab === "runtime" && <RuntimePanel project={project} />}
            {tab === "memory" && <MemoryPanel project={project} />}
          </div>
        </section>
      </div>
    </div>
  );
};

/* ------------------------------------------------------------------ shared bits */

const PanelBody: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="flex flex-col gap-4 px-4 pb-6 pt-4">{children}</div>
);

const Stat: React.FC<{ label: string; value: React.ReactNode }> = ({ label, value }) => (
  <div className="flex min-w-[72px] flex-col gap-1 rounded-lg border border-border bg-background px-3 py-2">
    <Label>{label}</Label>
    <span className="font-mono text-[18px] text-foreground">{value}</span>
  </div>
);

const Empty: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="px-4 py-6 text-center text-[12.5px] text-muted-foreground">{children}</div>
);

const STATE_ACCENT: Record<string, string> = {
  Planning: "bg-info",
  AwaitingApproval: "bg-warning",
  Running: "bg-primary",
  Validating: "bg-info",
  Review: "bg-violet",
  Completed: "bg-muted-foreground/40",
  Paused: "bg-warning",
  Cancelled: "bg-muted-foreground/30",
};

const SEGMENT: Record<string, string> = {
  Passed: "bg-primary",
  Skipped: "bg-primary/40",
  Executing: "bg-info animate-pulse",
  Judging: "bg-violet animate-pulse",
  Pending: "bg-muted",
};

/** One segment per milestone, coloured by its state: progress that also says what is happening. */
const MilestoneStrip: React.FC<{ mission: Mission }> = ({ mission }) => (
  <div className="flex gap-[3px]" aria-hidden="true">
    {mission.milestones.length === 0 && <div className="h-1.5 flex-1 rounded-full bg-muted" />}
    {mission.milestones.map((ms) => (
      <div key={ms.id} title={`${ms.title} · ${ms.state}`} className={cn("h-1.5 flex-1 rounded-full", SEGMENT[ms.state] ?? "bg-muted")} />
    ))}
  </div>
);

const ago = (iso: string) => formatAge(Math.max(0, Date.now() - new Date(iso).getTime()));

const MissionCard: React.FC<{ mission: Mission; onOpen: () => void }> = ({ mission, onOpen }) => {
  const passed = mission.milestones.filter((m) => m.state === "Passed" || m.state === "Skipped").length;
  const current = mission.milestones.find((m) => m.state === "Executing" || m.state === "Judging");
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group relative flex flex-col gap-2.5 overflow-hidden rounded-xl border border-border bg-card py-3 pl-5 pr-4 text-left transition-colors hover:border-primary/40 hover:bg-secondary/40"
    >
      <span className={cn("absolute inset-y-0 left-0 w-1", STATE_ACCENT[mission.state] ?? "bg-muted")} aria-hidden="true" />
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[13.5px] font-medium text-foreground">{mission.title}</span>
        <Pill tone={MISSION_TONE[mission.state] ?? "mute"} dot live={mission.state === "Running"}>
          {mission.state === "AwaitingApproval" ? "Awaiting approval" : mission.state}
        </Pill>
      </div>
      <MilestoneStrip mission={mission} />
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[10.5px] text-muted-foreground">
        <span>{passed}/{mission.milestones.length} milestones</span>
        {current && <span className="max-w-[200px] truncate text-info">{current.title}</span>}
        {mission.replans > 0 && <span className="text-warning">{mission.replans} re-plans</span>}
        <span className="ml-auto">${mission.cost.toFixed(2)} · {ago(mission.updated)}</span>
      </div>
    </button>
  );
};

/** The numbers that matter, always in view above the tabs. */
const SummaryStrip: React.FC<{ status: Status }> = ({ status }) => (
  <div className="flex shrink-0 flex-col gap-2.5 border-b border-border/70 px-4 py-3">
    <div className="flex items-center gap-3">
      <span className="min-w-0 flex-1 truncate text-[13px] text-foreground">{status.headline}</span>
      <span className="shrink-0 font-mono text-[11px] text-muted-foreground">{status.progress}%</span>
    </div>
    <div className="h-1 overflow-hidden rounded-full bg-muted">
      <div
        className="h-full rounded-full bg-gradient-to-r from-[#19e0a5] to-[#00b582] transition-[width]"
        style={{ width: `${status.progress}%` }}
      />
    </div>
    <div className="flex gap-5 font-mono text-[11px] text-muted-foreground">
      <span><b className="font-medium text-foreground">{status.live.length}</b> missions</span>
      <span><b className="font-medium text-foreground">{status.taskCount}</b> tasks</span>
      <span><b className="font-medium text-foreground">{status.milestonesPassed}/{status.milestonesTotal}</b> milestones</span>
      <span className="ml-auto"><b className="font-medium text-foreground">${status.cost.toFixed(2)}</b> spent</span>
    </div>
  </div>
);

/* ------------------------------------------------------------------ tabs */

const OverviewPanel: React.FC<{
  status: Status;
  onOpenMission: (id: string) => void;
  onOpenFull: (id: string) => void;
  onAskManager: () => void;
}> = ({ status, onOpenMission, onOpenFull, onAskManager }) => {
  const passedMs = status.missions.flatMap((m) => m.milestones).filter((m) => m.state === "Passed");
  const firstTry = passedMs.length
    ? Math.round((passedMs.filter((m) => m.attempts <= 1).length / passedMs.length) * 100)
    : null;
  const approvals = status.missions.filter((m) => m.state === "AwaitingApproval" || m.state === "Paused");
  const activity = status.missions
    .flatMap((m) => (m.log ?? []).map((entry) => ({ ...entry, mission: m.title })))
    .sort((x, y) => y.at.localeCompare(x.at))
    .slice(0, 12);

  return (
    <PanelBody>
      {approvals.length > 0 && (
        <Card
          title="Needs you"
          meta={<Pill tone="warn">{approvals.length}</Pill>}
          className="border-warning/30"
          bodyClassName="divide-y divide-border/60"
        >
          {approvals.map((m) => (
            <div key={m.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
              <div className="min-w-0 flex-1 basis-40">
                <div className="truncate text-[13px] font-medium text-foreground">{m.title}</div>
                <div className="truncate text-[12px] text-muted-foreground">
                  {m.state === "Paused" ? m.pauseReason || "Paused" : "Planned and waiting for approval."}
                </div>
              </div>
              {m.state === "AwaitingApproval" ? (
                <PrimaryButton size="sm" onClick={() => void bridge.missionAction(m.id, "approve")}>Approve</PrimaryButton>
              ) : (
                <GhostButton size="sm" onClick={() => void bridge.missionAction(m.id, "resume")}>Resume</GhostButton>
              )}
              <GhostButton size="sm" onClick={() => onOpenFull(m.id)}>Review</GhostButton>
            </div>
          ))}
        </Card>
      )}

      {status.attention.length > 0 && approvals.length === 0 && (
        <ul className="m-0 flex list-none flex-col gap-1.5 rounded-xl border border-border bg-card p-3.5 text-[12.5px] text-muted-foreground">
          {status.attention.slice(0, 4).map((line) => (
            <li key={line} className="flex gap-2">
              <span className="mt-1.5 text-warning"><Dot /></span>
              {line}
            </li>
          ))}
        </ul>
      )}

      {status.live.length === 0 && approvals.length === 0 && (
        <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border px-6 py-10 text-center">
          <span className="text-[13px] text-muted-foreground">Nothing is running. Tell the manager what you want built.</span>
          <GhostButton size="sm" className="lg:hidden" onClick={onAskManager}>Talk to manager</GhostButton>
        </div>
      )}

      {status.live.map((m) => (
        <Card
          key={m.id}
          title={m.title}
          meta={<Pill tone={MISSION_TONE[m.state] ?? "mute"} dot live={m.state === "Running"}>{m.state}</Pill>}
          actions={<GhostButton size="sm" onClick={() => onOpenMission(m.id)}>Open</GhostButton>}
          bodyClassName="gap-3 p-4"
        >
          <MissionGraph mission={m} compact />
        </Card>
      ))}

      <div className="flex flex-wrap gap-2">
        {firstTry !== null && <Stat label="First-try pass" value={`${firstTry}%`} />}
        {status.missions.length > 0 && (
          <Stat label="Avg mission" value={`$${(status.cost / status.missions.length).toFixed(2)}`} />
        )}
        <Stat label="Missions done" value={status.missions.filter((m) => m.state === "Completed").length} />
      </div>

      {activity.length > 0 && (
        <Card title="Activity" bodyClassName="px-4 py-2">
          <ol className="m-0 flex list-none flex-col p-0">
            {activity.map((entry, i) => (
              <li key={`${entry.at}-${i}`} className="relative flex gap-3 py-2 pl-4 text-[12.5px]">
                <span className="absolute left-0 top-[15px] size-1.5 rounded-full bg-border" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <div className="text-foreground">{entry.message}</div>
                  <div className="truncate font-mono text-[10.5px] text-muted-foreground">{entry.mission}</div>
                </div>
                <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{ago(entry.at)}</span>
              </li>
            ))}
          </ol>
        </Card>
      )}
    </PanelBody>
  );
};

type MissionFilter = "active" | "attention" | "done" | "cancelled" | "all";

const FILTERS: { value: MissionFilter; label: string; test: (m: Mission) => boolean }[] = [
  { value: "active", label: "Active", test: (m) => isLiveMission(m) },
  { value: "attention", label: "Needs you", test: (m) => m.state === "AwaitingApproval" || m.state === "Paused" },
  { value: "done", label: "Done", test: (m) => m.state === "Completed" },
  { value: "cancelled", label: "Cancelled", test: (m) => m.state === "Cancelled" },
  { value: "all", label: "All", test: () => true },
];

const MissionsPanel: React.FC<{ project: ProjectSummary; missions: Mission[]; onOpen: (id: string) => void }> = ({
  project,
  missions,
  onOpen,
}) => {
  const [filter, setFilter] = React.useState<MissionFilter>("active");
  const mine = React.useMemo(() => missions.filter((m) => m.project === project.name), [missions, project.name]);
  const counts = (test: (m: Mission) => boolean) => mine.filter(test).length;
  const active = FILTERS.find((f) => f.value === filter) ?? FILTERS[0];
  const shown = mine.filter(active.test).sort((a, b) => b.updated.localeCompare(a.updated));

  return (
    <PanelBody>
      <div className="flex flex-wrap gap-1.5">
        {FILTERS.map((f) => (
          <button
            key={f.value}
            type="button"
            onClick={() => setFilter(f.value)}
            aria-pressed={filter === f.value}
            className={cn(
              "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-[12px] transition-colors",
              filter === f.value
                ? "border-primary/50 bg-primary/12 text-foreground"
                : "border-border text-muted-foreground hover:text-foreground",
            )}
          >
            {f.label}
            <span className="font-mono text-[10.5px] text-muted-foreground">{counts(f.test)}</span>
          </button>
        ))}
      </div>

      {shown.length === 0 && (
        <div className="rounded-xl border border-dashed border-border px-6 py-10 text-center text-[12.5px] text-muted-foreground">
          {mine.length === 0 ? "No missions yet. Ask the manager to start one." : `No ${active.label.toLowerCase()} missions.`}
        </div>
      )}
      <div className="flex flex-col gap-2.5">
        {shown.map((m) => (
          <MissionCard key={m.id} mission={m} onOpen={() => onOpen(m.id)} />
        ))}
      </div>
    </PanelBody>
  );
};

const TasksPanel: React.FC<{ status: Status; onOpenJob: (id: string) => void }> = ({ status, onOpenJob }) => (
  <PanelBody>
    <Card title="Running tasks" meta={<Pill>{status.taskCount}</Pill>} bodyClassName="divide-y divide-border/60">
      {status.taskCount === 0 && <Empty>Nothing running.</Empty>}
      {status.managerWorking && (
        <div className="flex items-center gap-2.5 px-4 py-2.5">
          <span className="text-success"><Dot live /></span>
          <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground">Manager is working in chat</span>
          <Pill tone="ok" dot live>Running</Pill>
        </div>
      )}
      {status.runningJobs.map((job) => (
        <button
          key={job.id}
          type="button"
          onClick={() => onOpenJob(job.id)}
          className="flex items-center gap-2.5 px-4 py-2.5 text-left hover:bg-secondary/50"
        >
          <SquareTerminal className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground">
            {job.planTitle ?? job.prompt ?? job.type}
          </span>
          <Pill tone={job.status === "Running" ? "ok" : "mute"} dot live={job.status === "Running"}>
            {job.status}
          </Pill>
        </button>
      ))}
    </Card>
  </PanelBody>
);

const RuntimePanel: React.FC<{ project: ProjectSummary }> = ({ project }) => {
  const forwards = usePortForwards();
  const docker = useAsync<ProjectDocker>(() => bridge.getProjectDocker(project.name), [project.name]);
  return (
    <PanelBody>
      <Card title="Ports" meta={<Network className="size-3.5 text-muted-foreground" aria-hidden="true" />} bodyClassName="divide-y divide-border/60">
        {forwards.length === 0 && <Empty>No forwarded ports.</Empty>}
        {forwards.map((f) => (
          <div key={f.remotePort} className="flex items-center gap-3 px-4 py-2.5 font-mono text-[12px]">
            <span className="text-foreground">:{f.remotePort}</span>
            <span className="text-muted-foreground">{f.forwarded ? `→ localhost:${f.localPort}` : "local"}</span>
          </div>
        ))}
      </Card>
      <Card title="Docker" meta={<Box className="size-3.5 text-muted-foreground" aria-hidden="true" />} bodyClassName="divide-y divide-border/60">
        {docker && !docker.available && (
          <div className="flex flex-col gap-1 px-4 py-5 text-[12.5px]">
            <span className="text-foreground">Docker isn't reachable from the daemon.</span>
            <span className="text-muted-foreground">{docker.reason || "Docker is not available."}</span>
          </div>
        )}
        {docker?.available && docker.containers.length === 0 && <Empty>No containers for this project.</Empty>}
        {docker?.containers.map((c) => (
          <div key={c.id} className="flex items-center gap-3 px-4 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="truncate text-[12.5px] text-foreground">{c.name}</div>
              <div className="truncate font-mono text-[10.5px] text-muted-foreground">{c.image} {c.ports}</div>
            </div>
            <Pill tone={c.state === "running" ? "ok" : "mute"} dot live={c.state === "running"}>{c.state}</Pill>
          </div>
        ))}
      </Card>
    </PanelBody>
  );
};

const MemoryPanel: React.FC<{ project: ProjectSummary }> = ({ project }) => {
  const memory = useAsync<ProjectMemoryEntry[]>(() => bridge.listProjectMemory(project.name), [project.name], 60_000);
  return (
    <PanelBody>
      <Card title="Memory" meta={<Brain className="size-3.5 text-muted-foreground" aria-hidden="true" />} bodyClassName="divide-y divide-border/60">
        {memory && memory.length === 0 && <Empty>No memory files yet.</Empty>}
        {memory?.map((m) => (
          <div key={m.fileName} className="flex items-baseline gap-3 px-4 py-2.5">
            <span className="shrink-0 text-[12.5px] text-foreground">{m.title || m.fileName}</span>
            <span className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">{m.snippet}</span>
          </div>
        ))}
      </Card>
    </PanelBody>
  );
};

/* ------------------------------------------------------------- manager chat */

/** What is worth asking a project manager, in place of the generic chat's suggestions. */
const MANAGER_PROMPTS = [
  { label: "What's the state of things?", prompt: "Give me a quick status: what's running, what's stuck, and what needs me." },
  { label: "What should we build next?", prompt: "Looking at this project, what are the most valuable next things to build? Suggest three and recommend one." },
  { label: "Check my open PRs", prompt: "Check my open pull requests and their CI. Fix anything that's red by delegating it, and tell me what you did." },
  { label: "Tidy up the repo", prompt: "Look for stale branches, old worktrees and finished missions that can be cleaned up, and tell me what you'd remove." },
];

const MANAGER_COMMANDS = [{ name: "/clear", description: "Start the conversation over" }];

/** The briefing and engine notes are instructions the manager reads, not part of the conversation. */
const isManagerNote = (m: ChatMessage): boolean =>
  m.role === "system" &&
  (m.content.startsWith("# You are the Factory Manager") || m.content.startsWith("Engine switched by the operator"));

const ManagerChat: React.FC<{
  project: ProjectSummary;
  onOpenPlan: (planId: string) => void;
  draft?: { text: string; token: number };
}> = ({ project, onOpenPlan, draft }) => {
  const [error, setError] = React.useState<string | null>(null);
  const [ready, setReady] = React.useState(false);
  const sessionId = React.useRef<string | null>(null);

  /** `/clear` starts the manager's conversation over: the session is replaced, briefing included. */
  const onCommand = React.useCallback(
    async (text: string): Promise<boolean> => {
      if (text.trim().toLowerCase() !== "/clear") return false;
      const id = sessionId.current;
      if (!id) return true;
      try {
        if (chatStore.isSessionGenerating(id)) await chatStore.cancelGeneration();
        await chatStore.deleteSession(id);
        const fresh = await bridge.getProjectManager(project.name);
        sessionId.current = fresh.id;
        await chatStore.fetchSessions();
        await chatStore.selectSession(fresh.id);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
      return true;
    },
    [project.name],
  );

  React.useEffect(() => {
    let live = true;
    setReady(false);
    setError(null);
    (async () => {
      try {
        const session = await bridge.getProjectManager(project.name);
        sessionId.current = session.id;
        // `init` already lists the sessions the first time; asking again on every open re-downloads
        // every conversation in full just to show one.
        await chatStore.init();
        await chatStore.selectSession(session.id);
        if (live) setReady(true);
      } catch (e) {
        if (live) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      live = false;
    };
  }, [project.name]);

  if (error) return <div className="p-6 text-[13px] text-destructive">Could not open the manager: {error}</div>;
  if (!ready) return <div className="p-6 text-[13px] text-muted-foreground">Opening the manager…</div>;
  return (
    <ChatView
      embedded
      onOpenPlan={onOpenPlan}
      greeting={`Manager · ${project.name}`}
      headline="What should we build?"
      hideMessage={isManagerNote}
      onCommand={onCommand}
      commands={MANAGER_COMMANDS}
      samplePrompts={MANAGER_PROMPTS}
      draftPrompt={draft}
    />
  );
};
