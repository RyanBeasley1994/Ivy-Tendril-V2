import { useSyncExternalStore } from "react";

/**
 * Support tickets minimised to the bottom of the window, like chat heads: each docked ticket is a
 * bubble, and one at a time opens as a small floating window over whatever page is showing.
 *
 * Kept in local storage: which tickets an operator has docked is a per-device convenience.
 */

export interface DockedTicket {
  id: string;
  subject: string;
  partner?: string | null;
}

interface DockState {
  docked: DockedTicket[];
  openId: string | null;
}

const KEY = "tendril.supportDock";
const MAX_DOCKED = 6;

const read = (): DockState => {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as DockState;
      return { docked: parsed.docked ?? [], openId: null };
    }
  } catch {
    // Private window or blocked storage: start empty.
  }
  return { docked: [], openId: null };
};

let state: DockState = read();
const listeners = new Set<() => void>();

const set = (next: DockState) => {
  state = next;
  try {
    localStorage.setItem(KEY, JSON.stringify({ docked: next.docked }));
  } catch {
    // Not persisted; still works for this session.
  }
  for (const l of listeners) l();
};

export const supportDock = {
  /** Minimises a ticket into a bubble (newest first), leaving it closed. */
  dock(ticket: DockedTicket) {
    const docked = [ticket, ...state.docked.filter((t) => t.id !== ticket.id)].slice(0, MAX_DOCKED);
    set({ docked, openId: null });
  },
  open(id: string) {
    set({ ...state, openId: state.openId === id ? null : id });
  },
  collapse() {
    set({ ...state, openId: null });
  },
  remove(id: string) {
    set({ docked: state.docked.filter((t) => t.id !== id), openId: state.openId === id ? null : state.openId });
  },
};

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

export const useSupportDock = (): DockState => useSyncExternalStore(subscribe, () => state, () => state);
