import { useEffect, useState } from "react";
import { bridge } from "../api/bridge";

/**
 * Which plans belong to a mission: every milestone's plan and the integration plan. Missions run
 * their own plans, so the Plans page leaves them out and they are followed from Missions instead.
 *
 * The plan summary does not carry its mission link, but every mission names its plans, so the set is
 * read from the mission list. One poll for the whole app, shared by every subscriber.
 */

const POLL_MS = 10_000;

let ids: ReadonlySet<string> = new Set();
/** Only the milestones' plans: the integration plan is the mission's deliverable, reviewed as usual. */
let milestoneIds: ReadonlySet<string> = new Set();
/** Integration plan id → its mission's id, so review actions on that plan can go to the mission. */
let integrationMissions: ReadonlyMap<string, string> = new Map();
const listeners = new Set<(next: ReadonlySet<string>) => void>();
const milestoneListeners = new Set<(next: ReadonlySet<string>) => void>();
let timer: number | null = null;

/** `00042-AddLogin` → `42`, so `00042` and `42` compare equal. */
const normalize = (idOrFolder: string): string => {
  const n = Number.parseInt(idOrFolder.slice(0, 5), 10);
  return Number.isNaN(n) ? idOrFolder.toLowerCase() : String(n);
};

async function read(): Promise<void> {
  try {
    const missions = await bridge.listMissions();
    const next = new Set<string>();
    const nextMilestones = new Set<string>();
    const nextIntegration = new Map<string, string>();
    for (const m of missions) {
      if (m.integrationPlan) {
        next.add(normalize(m.integrationPlan));
        nextIntegration.set(normalize(m.integrationPlan), m.id);
      }
      for (const ms of m.milestones) {
        if (!ms.plan) continue;
        next.add(normalize(ms.plan));
        nextMilestones.add(normalize(ms.plan));
      }
    }
    integrationMissions = nextIntegration;
    const differs = (a: ReadonlySet<string>, b: ReadonlySet<string>) =>
      a.size !== b.size || [...a].some((id) => !b.has(id));
    if (differs(nextMilestones, milestoneIds)) {
      milestoneIds = nextMilestones;
      for (const l of milestoneListeners) l(milestoneIds);
    }
    if (!differs(next, ids)) return;
    ids = next;
    for (const l of listeners) l(ids);
  } catch {
    // No missions route (an older daemon), or offline: nothing is hidden.
  }
}

/** Whether a plan id belongs to a mission, against the latest set. */
export const isMissionPlan = (set: ReadonlySet<string>, planId: string): boolean =>
  set.has(normalize(planId));

export function useMissionPlanIds(): ReadonlySet<string> {
  const [value, setValue] = useState(ids);
  useEffect(() => {
    listeners.add(setValue);
    if (timer === null) {
      void read();
      timer = window.setInterval(() => void read(), POLL_MS);
    }
    return () => {
      listeners.delete(setValue);
      if (listeners.size === 0 && milestoneListeners.size === 0 && timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
  }, []);
  return value;
}

/**
 * The plans mission milestones run on. A milestone that fails is the mission's to retry, judge or
 * pause on, so it stays out of Review and the dashboard's decisions; the mission surfaces anything
 * that needs a human. Shares the mission poll with {@link useMissionPlanIds}.
 */
export function useMilestonePlanIds(): ReadonlySet<string> {
  const [value, setValue] = useState(milestoneIds);
  useEffect(() => {
    milestoneListeners.add(setValue);
    setValue(milestoneIds);
    if (timer === null) {
      void read();
      timer = window.setInterval(() => void read(), POLL_MS);
    }
    return () => {
      milestoneListeners.delete(setValue);
      if (listeners.size === 0 && milestoneListeners.size === 0 && timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
  }, []);
  return value;
}

/**
 * The mission whose integration plan `planId` is, if any. Read from the last mission poll; a review
 * action on that plan (Request Changes) belongs to the mission, not to a plain plan retry.
 */
export async function missionForIntegrationPlan(planId: string): Promise<string | null> {
  if (integrationMissions.size === 0) await read();
  return integrationMissions.get(normalize(planId)) ?? null;
}
