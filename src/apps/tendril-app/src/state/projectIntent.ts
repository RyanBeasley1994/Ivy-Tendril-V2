/**
 * A one-shot request to land on a particular panel (and optionally a mission) of a project page.
 * The palette sets it, then navigates; the page takes it on mount, or immediately if already open.
 */
export type ProjectTab = "overview" | "missions" | "tasks" | "runtime" | "memory";

export interface ProjectIntent {
  tab?: ProjectTab;
  missionId?: string;
}

let pending: ProjectIntent | null = null;
const listeners = new Set<() => void>();

export const setProjectIntent = (intent: ProjectIntent): void => {
  pending = intent;
  listeners.forEach((l) => l());
};

export const takeProjectIntent = (): ProjectIntent | null => {
  const taken = pending;
  pending = null;
  return taken;
};

export const subscribeProjectIntent = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};
