import { useSyncExternalStore } from "react";

/** The projects opened most recently, newest first, kept in this browser for the "Jump back in" row. */
const KEY = "tendril.recentProjects";
const MAX = 6;

export interface RecentProject {
  name: string;
  at: number;
}

let cache: RecentProject[] | null = null;
const listeners = new Set<() => void>();

const read = (): RecentProject[] => {
  if (cache) return cache;
  try {
    const raw = localStorage.getItem(KEY);
    const parsed = raw ? (JSON.parse(raw) as RecentProject[]) : [];
    cache = Array.isArray(parsed) ? parsed.filter((r) => typeof r?.name === "string") : [];
  } catch {
    cache = [];
  }
  return cache;
};

export const recordRecentProject = (name: string): void => {
  const next = [{ name, at: Date.now() }, ...read().filter((r) => r.name !== name)].slice(0, MAX);
  cache = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* storage unavailable: the list just lasts for this session */
  }
  listeners.forEach((l) => l());
};

export const useRecentProjects = (): RecentProject[] =>
  useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    read,
    read,
  );
