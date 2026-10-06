import type { Job, Mission } from "../types/api";

const LIVE_STATES = new Set(["Planning", "AwaitingApproval", "Running", "Validating", "Review", "Paused"]);

export const isLiveMission = (m: Mission): boolean => LIVE_STATES.has(m.state);

export interface ProjectStatus {
  missions: Mission[];
  live: Mission[];
  jobs: Job[];
  runningJobs: Job[];
  milestonesTotal: number;
  milestonesPassed: number;
  /** 0-100, across every non-cancelled mission. */
  progress: number;
  cost: number;
  /** One line a person would say about where things stand, built only from real mission state. */
  headline: string;
  attention: string[];
  /** What is being worked on right now: the active milestone, else a running task. */
  currentTask: string | null;
  /** Latest mission update, RFC 3339; null when the project has no missions. */
  updated: string | null;
  /** Anything running at all, mission or task, or the manager mid-turn. */
  busy: boolean;
  /** The manager is in the middle of a chat turn (it may be doing nothing a mission or job shows). */
  managerWorking: boolean;
  /** Running jobs, plus the manager's own turn. */
  taskCount: number;
}

export const projectStatus = (
  project: string,
  allMissions: Mission[],
  allJobs: Job[],
  managerWorking = false,
): ProjectStatus => {
  const missions = allMissions.filter((m) => m.project === project && m.state !== "Cancelled");
  const live = missions.filter(isLiveMission);
  const jobs = allJobs.filter((j) => j.project === project);
  const runningJobs = jobs.filter((j) => j.status === "Running" || j.status === "Queued");
  const milestones = missions.flatMap((m) => m.milestones);
  const milestonesPassed = milestones.filter((m) => m.state === "Passed" || m.state === "Skipped").length;
  const progress = milestones.length ? Math.round((milestonesPassed / milestones.length) * 100) : 0;

  const attention: string[] = [];
  for (const m of missions) {
    if (m.state === "AwaitingApproval") attention.push(`"${m.title}" is waiting for approval.`);
    if (m.state === "Paused") attention.push(`"${m.title}" is paused${m.pauseReason ? `: ${m.pauseReason}` : "."}`);
    if (m.rateLimit) attention.push(`"${m.title}" is waiting out a rate limit.`);
    for (const ms of m.milestones) {
      if (ms.attempts > 1 && ms.state !== "Passed") {
        attention.push(`"${ms.title}" has needed ${ms.attempts} attempts; retrying automatically.`);
      }
    }
    if (m.replans > 0 && isLiveMission(m)) attention.push(`"${m.title}" has been re-planned ${m.replans}×.`);
  }

  let headline: string;
  if (managerWorking && !live.length && !runningJobs.length) headline = "The manager is working.";
  else if (!missions.length && !runningJobs.length) headline = "Idle. Tell the manager what you want built.";
  else if (!live.length && runningJobs.length) headline = `${runningJobs.length} task${runningJobs.length === 1 ? "" : "s"} running.`;
  else if (!live.length) headline = "Everything is complete.";
  else if (attention.length) headline = "Working, with things worth a look.";
  else headline = `${live.length} mission${live.length === 1 ? "" : "s"} running. No action required.`;

  const running = live.flatMap((m) => m.milestones.map((ms) => ({ m, ms }))).find(({ ms }) => ms.state === "Executing" || ms.state === "Judging");
  const currentTask =
    running?.ms.title ??
    (runningJobs[0] ? (runningJobs[0].planTitle ?? runningJobs[0].prompt ?? runningJobs[0].type) : null) ??
    (managerWorking ? "Manager is working in chat" : null) ??
    (live[0] ? live[0].title : null);
  const updated = missions.map((m) => m.updated).sort().at(-1) ?? null;

  return {
    currentTask,
    updated,
    busy: managerWorking || live.some((m) => m.state !== "Paused" && m.state !== "AwaitingApproval") || runningJobs.length > 0,
    managerWorking,
    taskCount: runningJobs.length + (managerWorking ? 1 : 0),
    missions,
    live,
    jobs,
    runningJobs,
    milestonesTotal: milestones.length,
    milestonesPassed,
    progress,
    cost: missions.reduce((sum, m) => sum + (m.cost ?? 0), 0),
    headline,
    attention,
  };
};
