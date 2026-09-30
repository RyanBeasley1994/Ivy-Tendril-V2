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
const listeners = new Set<(next: ReadonlySet<string>) => void>();
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
    for (const m of missions) {
      if (m.integrationPlan) next.add(normalize(m.integrationPlan));
      for (const ms of m.milestones) if (ms.plan) next.add(normalize(ms.plan));
    }
    const changed = next.size !== ids.size || [...next].some((id) => !ids.has(id));
    if (!changed) return;
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
      if (listeners.size === 0 && timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
  }, []);
  return value;
}
