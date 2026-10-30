import React from "react";
import type { ManagerActivity } from "./managerActivity";

/**
 * Which projects' managers have said something the operator has not seen: a finished turn — a task
 * reported back and the manager answered it — since the project was last open.
 *
 * "Seen" is when each manager's conversation last changed while its project was the open one, kept in
 * local storage per device. A project the store has never heard of starts as seen, so the first run
 * does not light up every project at once.
 */
const KEY = "tendril.managerSeen";

const read = (): Record<string, string> => {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, string>;
  } catch {
    return {};
  }
};

const write = (seen: Record<string, string>) => {
  try {
    localStorage.setItem(KEY, JSON.stringify(seen));
  } catch {
    // Private window or blocked storage: unread lasts for this session only.
  }
};

export const useManagerUnread = (activity: ManagerActivity, activeProject: string | null): Set<string> => {
  const [seen, setSeen] = React.useState(read);

  React.useEffect(() => {
    let changed = false;
    const next = { ...seen };
    for (const [project, a] of Object.entries(activity)) {
      if (!a.updatedAt) continue;
      if (next[project] === undefined || (project === activeProject && next[project] !== a.updatedAt)) {
        next[project] = a.updatedAt;
        changed = true;
      }
    }
    if (changed) {
      write(next);
      setSeen(next);
    }
  }, [activity, activeProject, seen]);

  return React.useMemo(() => {
    const unread = new Set<string>();
    for (const [project, a] of Object.entries(activity)) {
      const last = seen[project];
      if (project === activeProject || a.working || !a.updatedAt || !last) continue;
      if (Date.parse(a.updatedAt) > Date.parse(last)) unread.add(project);
    }
    return unread;
  }, [activity, activeProject, seen]);
};
