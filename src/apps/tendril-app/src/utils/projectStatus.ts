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
}

export const projectStatus = (project: string, allMissions: Mission[], allJobs: Job[]): ProjectStatus => {
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
  if (!missions.length && !runningJobs.length) headline = "Idle. Tell the manager what you want built.";
  else if (!live.length && runningJobs.length) headline = `${runningJobs.length} task${runningJobs.length === 1 ? "" : "s"} running.`;
  else if (!live.length) headline = "Everything is complete.";
  else if (attention.length) headline = "Working, with things worth a look.";
  else headline = `${live.length} mission${live.length === 1 ? "" : "s"} running. No action required.`;

  return {
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
