import React from "react";
import { ArrowLeft, Check, ExternalLink } from "lucide-react";
import type { Milestone, Mission } from "../../types/api";
import { bridge } from "../../api/bridge";
import { Card, Dot, GhostButton, Label, Pill, PrimaryButton, type Tone } from "../../components/page/kit";
import { cn } from "@ivy-interactive/components/ui";

type NodeState = "done" | "active" | "waiting" | "failed";

interface GraphNode {
  id: string;
  title: string;
  sub: string;
  state: NodeState;
}

const NODE_STYLE: Record<NodeState, string> = {
  done: "border-primary/40 bg-primary/10 text-foreground",
  active: "border-info/60 bg-info/10 text-foreground shadow-[0_0_0_3px_color-mix(in_srgb,var(--info)_18%,transparent)]",
  waiting: "border-border bg-background text-muted-foreground",
  failed: "border-destructive/50 bg-destructive/10 text-foreground",
};

const milestoneNode = (ms: Milestone): GraphNode => ({
  id: ms.id,
  title: ms.title,
  sub:
    ms.state === "Passed" || ms.state === "Skipped"
      ? "passed"
      : ms.state === "Pending"
        ? "waiting"
        : `${ms.state.toLowerCase()}${ms.attempts > 1 ? ` · attempt ${ms.attempts}` : ""}`,
  state:
    ms.state === "Passed" || ms.state === "Skipped"
      ? "done"
      : ms.state === "Pending"
        ? ms.feedback
          ? "failed"
          : "waiting"
        : "active",
});

/** Plan → each milestone → validation → review, coloured by where the mission actually is. */
export const buildGraph = (m: Mission): GraphNode[] => {
  const planned = m.milestones.length > 0 && m.state !== "Planning";
  return [
    {
      id: "plan",
      title: "Plan",
      sub: planned ? `${m.milestones.length} milestones` : "planning",
      state: planned ? "done" : "active",
    },
    ...m.milestones.map(milestoneNode),
    {
      id: "validate",
      title: "Validate",
      sub: m.state === "Validating" ? "running" : m.state === "Review" || m.state === "Completed" ? "done" : "waiting",
      state:
        m.state === "Validating"
          ? "active"
          : m.state === "Review" || m.state === "Completed"
            ? "done"
            : "waiting",
    },
    {
      id: "review",
      title: "Review",
      sub: m.state === "Completed" ? "merged" : m.state === "Review" ? "needs you" : "waiting",
      state: m.state === "Completed" ? "done" : m.state === "Review" ? "active" : "waiting",
    },
  ];
};

export const MissionGraph: React.FC<{ mission: Mission; compact?: boolean }> = ({ mission, compact }) => {
  const nodes = buildGraph(mission);
  return (
    <div className="flex items-stretch gap-0 overflow-x-auto pb-1">
      {nodes.map((node, i) => (
        <React.Fragment key={node.id}>
          <div
            className={cn(
              "flex shrink-0 flex-col justify-center gap-0.5 rounded-lg border px-3 py-2",
              compact ? "w-[120px]" : "w-[150px]",
              NODE_STYLE[node.state],
            )}
          >
            <span className="truncate text-[12px] font-medium">{node.title}</span>
            <span className="flex items-center gap-1.5 font-mono text-[10.5px] text-muted-foreground">
              {node.state === "active" && <span className="text-info"><Dot live /></span>}
              {node.state === "done" && <Check className="size-3 text-success" aria-hidden="true" />}
              {node.sub}
            </span>
          </div>
          {i < nodes.length - 1 && (
            <div className="flex w-5 shrink-0 items-center" aria-hidden="true">
              <div className={cn("h-px w-full", node.state === "done" ? "bg-primary/50" : "bg-border")} />
            </div>
          )}
        </React.Fragment>
      ))}
    </div>
  );
};

const STEP_TONE: Record<string, Tone> = {
  Plan: "info",
  Execute: "ok",
  Retry: "warn",
  Judge: "violet",
  Final: "info",
  Revise: "warn",
  Steer: "mute",
};

const fmtTime = (iso: string) => new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** One mission, in detail: the graph, what is running, the job trace and the manager's log. */
export const MissionRun: React.FC<{
  missionId: string;
  onBack: () => void;
  onOpenJob: (jobId: string) => void;
  onOpenFull: (missionId: string) => void;
}> = ({ missionId, onBack, onOpenJob, onOpenFull }) => {
  const [mission, setMission] = React.useState<Mission | null>(null);

  React.useEffect(() => {
    let live = true;
    const load = () =>
      bridge
        .getMission(missionId)
        .then((m) => live && setMission(m))
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, 5_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [missionId]);

  if (!mission) return <div className="p-6 text-[13px] text-muted-foreground">Loading mission…</div>;

  const passed = mission.milestones.filter((m) => m.state === "Passed" || m.state === "Skipped").length;
  const pct = mission.milestones.length ? Math.round((passed / mission.milestones.length) * 100) : 0;
  const jobs = mission.jobs ?? [];
  const log = [...(mission.log ?? [])].reverse();

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-5 pb-6 pt-4" data-testid="mission-run">
      <div className="flex items-center gap-3">
        <GhostButton size="sm" onClick={onBack}>
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          Overview
        </GhostButton>
        <h2 className="m-0 min-w-0 flex-1 truncate text-[17px] font-semibold text-foreground">{mission.title}</h2>
        <Pill tone={mission.state === "Running" ? "ok" : mission.state === "Paused" ? "warn" : "info"} dot live={mission.state === "Running"}>
          {mission.state}
        </Pill>
        <GhostButton size="sm" onClick={() => onOpenFull(mission.id)}>
          <ExternalLink className="size-3.5" aria-hidden="true" />
          Full mission view
        </GhostButton>
      </div>

      <Card title="Mission" bodyClassName="gap-3 p-4">
        <p className="m-0 text-[13px] leading-relaxed text-muted-foreground">{mission.goal}</p>
        <div className="h-1.5 overflow-hidden rounded-full bg-muted">
          <div className="h-full rounded-full bg-gradient-to-b from-[#19e0a5] to-[#00b582]" style={{ width: `${pct}%` }} />
        </div>
        <div className="flex gap-4 font-mono text-[11px] text-muted-foreground">
          <span>{passed}/{mission.milestones.length} milestones</span>
          <span>${mission.cost.toFixed(2)}{mission.budget.maxCost ? ` of $${mission.budget.maxCost}` : ""}</span>
          <span>{mission.replans}/{mission.budget.maxReplans} re-plans</span>
          {mission.branch && <span>{mission.branch}</span>}
        </div>
      </Card>

      <Card title="Live mission graph" meta={mission.state === "Running" ? <Pill tone="ok" dot live>LIVE</Pill> : undefined} bodyClassName="p-4">
        <MissionGraph mission={mission} />
      </Card>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        <Card title="Execution trace" meta={<Pill>{jobs.length}</Pill>} bodyClassName="divide-y divide-border/60">
          {jobs.length === 0 && <div className="px-4 py-6 text-center text-[12.5px] text-muted-foreground">No jobs yet.</div>}
          {jobs.map((j) => {
            const ms = mission.milestones.find((m) => m.id === j.milestone);
            const current = mission.currentJob?.jobId === j.jobId;
            return (
              <button
                key={j.jobId}
                type="button"
                onClick={() => onOpenJob(j.jobId)}
                className="flex items-center gap-3 px-4 py-2.5 text-left hover:bg-secondary/50"
              >
                <Pill tone={STEP_TONE[j.step] ?? "mute"}>{j.step}</Pill>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[12.5px] text-foreground">{ms?.title ?? "Mission"}</div>
                  {j.agent && <div className="truncate font-mono text-[10.5px] text-muted-foreground">{j.agent}</div>}
                </div>
                {current && <span className="text-success"><Dot live /></span>}
                <span className="font-mono text-[10.5px] text-muted-foreground">#{j.jobId}</span>
              </button>
            );
          })}
        </Card>

        <Card title="Manager log" meta={<Pill>{log.length}</Pill>} bodyClassName="divide-y divide-border/60">
          {log.length === 0 && <div className="px-4 py-6 text-center text-[12.5px] text-muted-foreground">Nothing logged yet.</div>}
          {log.slice(0, 40).map((entry, i) => (
            <div key={`${entry.at}-${i}`} className="flex gap-3 px-4 py-2 text-[12.5px]">
              <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">{fmtTime(entry.at)}</span>
              <span className="text-foreground">{entry.message}</span>
            </div>
          ))}
        </Card>
      </div>

      {mission.state === "AwaitingApproval" && (
        <div className="flex items-center gap-3 rounded-xl border border-warning/40 bg-warning/10 p-4">
          <Label>Waiting for you</Label>
          <span className="flex-1 text-[12.5px] text-foreground">Review the milestones above, then approve to let it run.</span>
          <PrimaryButton onClick={() => void bridge.missionAction(mission.id, "approve")}>Approve</PrimaryButton>
        </div>
      )}
    </div>
  );
};
