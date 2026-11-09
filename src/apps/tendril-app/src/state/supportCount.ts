import { useSyncExternalStore } from "react";
import { supportApi } from "../api/support";
import { playChime } from "../utils/chime";
import { notificationsStore } from "./notificationsStore";

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

/** A notification for what just came in: the newest unread ticket, by name when it can be found. */
const announce = async (added: number): Promise<void> => {
  const { tickets } = await supportApi.list(false).catch(() => ({ tickets: [] }));
  const newest = tickets
    .filter((t) => t.hasUnread)
    .sort((a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt))[0];
  const from = newest?.lastMessage?.authorName ?? newest?.partner?.name;
  const said = newest?.lastMessage?.content?.replace(/\s+/g, " ").trim() ?? "";
  notificationsStore.notifyManager({
    title: newest ? `Support · ${newest.subject}` : "Support",
    message: newest
      ? `${from ? `${from}: ` : ""}${said.length > 140 ? `${said.slice(0, 139)}…` : said}` || "New activity on this ticket."
      : `${added} new unread ticket${added === 1 ? "" : "s"}.`,
    isSuccess: true,
  });
};

export const refreshSupportCount = async (): Promise<void> => {
  try {
    const next = (await supportApi.count()).count;
    if (loaded && next > count) {
      playChime();
      void announce(next - count);
    }
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
