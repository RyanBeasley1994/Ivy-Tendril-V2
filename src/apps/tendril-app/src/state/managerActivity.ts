import { useSyncExternalStore } from "react";
import { bridge } from "../api/bridge";

/**
 * Which projects' managers are in the middle of a turn. One poller serves every subscriber (the
 * dashboard, the sidebar cards, the project cards, the palette) and runs only while one is mounted.
 */
export type ManagerActivity = Record<
  string,
  {
    working: boolean;
    updatedAt: string | null;
    /** Things waiting on the operator: missions to approve or resume, plus an unanswered question. */
    needsYou?: number;
    /** The manager's latest message asks something. */
    asked?: boolean;
  }
>;

const POLL_MS = 4_000;
const EMPTY: ManagerActivity = {};

let snapshot: ManagerActivity = EMPTY;
let timer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<() => void>();

const same = (a: ManagerActivity, b: ManagerActivity): boolean => {
  const keys = Object.keys(b);
  if (keys.length !== Object.keys(a).length) return false;
  return keys.every(
    (k) =>
      a[k]?.working === b[k].working &&
      a[k]?.updatedAt === b[k].updatedAt &&
      a[k]?.needsYou === b[k].needsYou &&
      a[k]?.asked === b[k].asked,
  );
};

const poll = () =>
  bridge
    .getManagersStatus()
    .then((next) => {
      if (same(snapshot, next)) return;
      snapshot = next;
      listeners.forEach((l) => l());
    })
    .catch(() => undefined);

const subscribe = (listener: () => void): (() => void) => {
  listeners.add(listener);
  if (!timer) {
    void poll();
    timer = setInterval(poll, POLL_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer) {
      clearInterval(timer);
      timer = null;
    }
  };
};

export const useManagerActivity = (): ManagerActivity =>
  useSyncExternalStore(subscribe, () => snapshot, () => EMPTY);

/** How many things across every project wait on the operator: what the Projects badge counts. */
export const useNeedsYouCount = (): number => {
  const activity = useManagerActivity();
  return Object.values(activity).reduce((sum, m) => sum + (m.needsYou ?? 0), 0);
};
