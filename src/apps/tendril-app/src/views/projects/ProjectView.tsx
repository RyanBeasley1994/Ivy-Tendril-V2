import React from "react";
import { ArrowLeft, Box, Brain, Network, SquareTerminal } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { Job, Mission, ProjectSummary } from "../../types/api";
import type { ProjectDocker, ProjectMemoryEntry } from "../../types/projectAssets";
import { bridge } from "../../api/bridge";
import { usePortForwards } from "../../api/portForwards";
import { chatStore } from "../../state/chatStore";
import { projectStatus } from "../../utils/projectStatus";
import { Card, Dot, GhostButton, Label, Pill, PrimaryButton, Seg, type Tone } from "../../components/page/kit";
import { ChatView } from "../ChatView";
import { useMissions } from "./ProjectsHomeView";
import { MissionGraph, MissionRun } from "./MissionRun";

interface Props {
  project: ProjectSummary;
  jobs: Job[];
  onBack: () => void;
  onOpenMission: (missionId: string) => void;
  onOpenJob: (jobId: string) => void;
  onOpenPlan: (planId: string) => void;
}

type Status = ReturnType<typeof projectStatus>;
type Tab = "overview" | "missions" | "tasks" | "runtime" | "memory";
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
export const ProjectView: React.FC<Props> = ({ project, jobs, onBack, onOpenMission, onOpenJob, onOpenPlan }) => {
  const [tab, setTab] = React.useState<Tab>("overview");
  const [side, setSide] = React.useState<Side>("chat");
  const [runId, setRunId] = React.useState<string | null>(null);
  const missions = useMissions(10_000);
  const status = React.useMemo(() => projectStatus(project.name, missions, jobs), [project.name, missions, jobs]);

  const openRun = (id: string) => {
    setTab("missions");
    setSide("panels");
    setRunId(id);
  };
  const pickTab = (next: Tab) => {
    setTab(next);
    if (next !== "missions") setRunId(null);
  };

  const tabs: { value: Tab; label: React.ReactNode }[] = [
    { value: "overview", label: "Overview" },
    { value: "missions", label: `Missions${status.live.length ? ` · ${status.live.length}` : ""}` },
    { value: "tasks", label: `Tasks${status.runningJobs.length ? ` · ${status.runningJobs.length}` : ""}` },
    { value: "runtime", label: "Runtime" },
    { value: "memory", label: "Memory" },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="project-view">
      <header className="flex shrink-0 items-center gap-3 border-b border-border/70 px-5 py-3">
        <GhostButton size="sm" onClick={onBack} aria-label="Back to projects">
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Projects
        </GhostButton>
        <h1 className="m-0 text-[18px] font-semibold tracking-[-0.02em] text-foreground">{project.name}</h1>
        <Pill tone={status.live.length || status.runningJobs.length ? "ok" : "mute"} dot live={status.live.length > 0}>
          {status.live.length || status.runningJobs.length ? "Working" : "Idle"}
        </Pill>
        {/* Below `lg` there is room for one half at a time. */}
        <Seg<Side>
          className="ml-auto lg:hidden"
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
          <ManagerChat project={project} onOpenPlan={onOpenPlan} />
        </section>

        <section
          className={cn("min-w-0 flex-col lg:flex lg:w-1/2", side === "panels" ? "flex flex-1" : "hidden")}
          aria-label="Project panels"
        >
          <div className="shrink-0 overflow-x-auto border-b border-border/70 px-4 py-2">
            <Seg<Tab> label="Project panels" value={tab} onChange={pickTab} options={tabs} className="w-max" />
          </div>
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
                <MissionsPanel status={status} onOpen={openRun} />
              ))}
            {tab === "tasks" && <TasksPanel status={status} onOpenJob={onOpenJob} />}
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

const MissionRow: React.FC<{ mission: Mission; onOpen: () => void }> = ({ mission, onOpen }) => {
  const passed = mission.milestones.filter((m) => m.state === "Passed" || m.state === "Skipped").length;
  const pct = mission.milestones.length ? Math.round((passed / mission.milestones.length) * 100) : 0;
  return (
    <button type="button" onClick={onOpen} className="flex flex-col gap-1.5 px-4 py-3 text-left hover:bg-secondary/50">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground">{mission.title}</span>
        <Pill tone={MISSION_TONE[mission.state] ?? "mute"} dot live={mission.state === "Running"}>
          {mission.state}
        </Pill>
      </div>
      <div className="h-1 overflow-hidden rounded-full bg-muted">
        <div className="h-full rounded-full bg-primary" style={{ width: `${pct}%` }} />
      </div>
      <div className="flex gap-3 font-mono text-[10.5px] text-muted-foreground">
        <span>{passed}/{mission.milestones.length} milestones</span>
        {mission.replans > 0 && <span className="text-warning">{mission.replans} re-plans</span>}
        <span>${mission.cost.toFixed(2)}</span>
      </div>
    </button>
  );
};

/* ------------------------------------------------------------------ tabs */

const OverviewPanel: React.FC<{
  status: Status;
  onOpenMission: (id: string) => void;
  onOpenFull: (id: string) => void;
  onAskManager: () => void;
}> = ({ status, onOpenMission, onOpenFull, onAskManager }) => {
  const activeMission = status.live[0] ?? status.missions[0];
  const passedMs = status.missions.flatMap((m) => m.milestones).filter((m) => m.state === "Passed");
  const firstTry = passedMs.length
    ? Math.round((passedMs.filter((m) => m.attempts <= 1).length / passedMs.length) * 100)
    : null;
  const approvals = status.missions.filter((m) => m.state === "AwaitingApproval" || m.state === "Paused");

  return (
    <PanelBody>
      <Card
        title="Manager"
        actions={
          <GhostButton size="sm" className="lg:hidden" onClick={onAskManager}>
            Talk to manager
          </GhostButton>
        }
        bodyClassName="gap-4 p-4"
      >
        <div>
          <div className="text-[15px] font-semibold text-foreground">{status.headline}</div>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-gradient-to-b from-[#19e0a5] to-[#00b582] transition-[width]"
              style={{ width: `${status.progress}%` }}
            />
          </div>
          <div className="mt-1.5 flex justify-between font-mono text-[10.5px] text-muted-foreground">
            <span>{status.milestonesPassed}/{status.milestonesTotal} milestones</span>
            <span>{status.progress}%</span>
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          <Stat label="Missions" value={status.live.length} />
          <Stat label="Tasks" value={status.runningJobs.length} />
          <Stat label="Spend" value={`$${status.cost.toFixed(2)}`} />
          {firstTry !== null && <Stat label="First-try pass" value={`${firstTry}%`} />}
          {status.missions.length > 0 && (
            <Stat label="Avg mission" value={`$${(status.cost / status.missions.length).toFixed(2)}`} />
          )}
        </div>

        {status.attention.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-1 p-0 text-[12.5px] text-muted-foreground">
            {status.attention.slice(0, 4).map((line) => (
              <li key={line} className="flex gap-2">
                <span className="text-warning"><Dot /></span>
                {line}
              </li>
            ))}
          </ul>
        )}

        {activeMission && (
          <div className="flex flex-col gap-2 border-t border-border/70 pt-3">
            <button type="button" onClick={() => onOpenMission(activeMission.id)} className="w-fit text-left">
              <Label>{activeMission.title}</Label>
            </button>
            <MissionGraph mission={activeMission} compact />
          </div>
        )}
      </Card>

      {approvals.length > 0 && (
        <Card
          title="Needs you"
          meta={<Pill tone="warn">{approvals.length} pending</Pill>}
          bodyClassName="divide-y divide-border/60"
        >
          {approvals.map((m) => (
            <div key={m.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
              <div className="min-w-0 flex-1 basis-40">
                <div className="truncate text-[13px] font-medium text-foreground">{m.title}</div>
                <div className="truncate text-[12px] text-muted-foreground">
                  {m.state === "Paused" ? m.pauseReason || "Paused" : "Milestones are planned and waiting for approval."}
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
    </PanelBody>
  );
};

const MissionsPanel: React.FC<{ status: Status; onOpen: (id: string) => void }> = ({ status, onOpen }) => (
  <PanelBody>
    <Card title="Missions" meta={<Pill>{status.missions.length}</Pill>} bodyClassName="divide-y divide-border/60">
      {status.missions.length === 0 && <Empty>No missions yet. Ask the manager to start one.</Empty>}
      {status.missions.map((m) => (
        <MissionRow key={m.id} mission={m} onOpen={() => onOpen(m.id)} />
      ))}
    </Card>
  </PanelBody>
);

const TasksPanel: React.FC<{ status: Status; onOpenJob: (id: string) => void }> = ({ status, onOpenJob }) => (
  <PanelBody>
    <Card title="Running tasks" meta={<Pill>{status.runningJobs.length}</Pill>} bodyClassName="divide-y divide-border/60">
      {status.runningJobs.length === 0 && <Empty>Nothing running.</Empty>}
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
        {docker && !docker.available && <Empty>{docker.reason || "Docker is not available."}</Empty>}
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

const ManagerChat: React.FC<{ project: ProjectSummary; onOpenPlan: (planId: string) => void }> = ({
  project,
  onOpenPlan,
}) => {
  const [error, setError] = React.useState<string | null>(null);
  const [ready, setReady] = React.useState(false);

  React.useEffect(() => {
    let live = true;
    setReady(false);
    setError(null);
    (async () => {
      try {
        const session = await bridge.getProjectManager(project.name);
        await chatStore.init();
        await chatStore.fetchSessions();
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
    />
  );
};
