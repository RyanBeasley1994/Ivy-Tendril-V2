import { useSyncExternalStore } from "react";
import { supportApi } from "../api/support";
import { playChime } from "../utils/chime";

/**
 * The Support badge: the operator's unread tickets in "Needs Us", as Operations counts them. One
 * poller for every subscriber; [`refreshSupportCount`] is called after a ticket is read or moved so
 * the badge drops at once rather than on the next tick. A chime plays when the count goes up.
 */

const POLL_MS = 30_000;

let count = 0;
/** The first count is what was already there, not something arriving. */
let loaded = false;
let timer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<() => void>();

export const refreshSupportCount = async (): Promise<void> => {
  try {
    const next = (await supportApi.count()).count;
    if (loaded && next > count) playChime();
    loaded = true;
    if (next !== count) {
      count = next;
      for (const l of listeners) l();
    }
  } catch {
    // Unreachable Operations: keep the last count rather than flash the badge away.
  }
};

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  if (!timer) {
    void refreshSupportCount();
    timer = setInterval(() => void refreshSupportCount(), POLL_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer) {
      clearInterval(timer);
      timer = null;
    }
  };
};

export const useSupportCount = (): number => useSyncExternalStore(subscribe, () => count, () => 0);
