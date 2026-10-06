import React from "react";
import type { Job, ProjectSummary } from "../../types/api";
import { projectStatus } from "../../utils/projectStatus";
import { Dot } from "../../components/page/kit";
import { useMissions } from "../projects/ProjectsHomeView";
import { useManagerActivity } from "../../state/managerActivity";

interface Props {
  projects: ProjectSummary[];
  jobs: Job[];
  /** The project page currently open, so its card reads as selected. */
  activeProject: string | null;
  onOpenProject: (name: string) => void;
}

const MAX_CARDS = 4;

/**
 * One status card per project whose manager is working: what it is doing right now and how far it
 * has got. Click to jump into that project. Nothing is drawn while every manager is idle.
 */
export const SidebarManagers: React.FC<Props> = ({ projects, jobs, activeProject, onOpenProject }) => {
  const missions = useMissions(10_000);
  const activity = useManagerActivity();

  const busy = React.useMemo(
    () =>
      projects
        .map((project) => ({ project, status: projectStatus(project.name, missions, jobs, activity[project.name]?.working ?? false) }))
        .filter(({ status }) => status.busy || status.attention.length > 0)
        .sort(
          (a, b) =>
            Number(b.status.busy) - Number(a.status.busy) ||
            (b.status.updated ?? "").localeCompare(a.status.updated ?? ""),
        ),
    [projects, missions, jobs, activity],
  );

  if (busy.length === 0) return null;
  const shown = busy.slice(0, MAX_CARDS);

  return (
    <div className="tcc-sidebar-managers mt-3 flex shrink-0 flex-col gap-1.5" data-testid="sidebar-managers">
      <div className="px-2.5 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
        Managers · {busy.length}
      </div>
      {shown.map(({ project, status }) => (
        <button
          key={project.name}
          type="button"
          onClick={() => onOpenProject(project.name)}
          aria-current={activeProject === project.name ? "page" : undefined}
          className={`flex flex-col gap-1.5 rounded-[11px] border bg-card p-2.5 text-left transition-colors hover:border-primary/40 ${
            activeProject === project.name ? "border-primary/50" : "border-border"
          }`}
        >
          <div className="flex items-center gap-2">
            <span className={status.busy ? "text-success" : "text-warning"}>
              <Dot live={status.busy} />
            </span>
            <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-foreground">{project.name}</span>
            {status.live.length > 0 && (
              <span className="font-mono text-[10.5px] text-muted-foreground">{status.progress}%</span>
            )}
          </div>
          <div className="truncate text-[11.5px] text-muted-foreground">
            {status.currentTask ?? (status.attention[0] ?? status.headline)}
          </div>
          <div className="h-1 overflow-hidden rounded-full bg-secondary">
            <div className="h-full rounded-full bg-primary transition-[width]" style={{ width: `${status.progress}%` }} />
          </div>
          <div className="flex gap-3 font-mono text-[10px] text-muted-foreground">
            <span>{status.live.length} missions</span>
            <span>{status.taskCount} tasks</span>
            {status.attention.length > 0 && <span className="text-warning">needs you</span>}
          </div>
        </button>
      ))}
      {busy.length > MAX_CARDS && (
        <div className="px-2.5 font-mono text-[10.5px] text-muted-foreground">+{busy.length - MAX_CARDS} more</div>
      )}
    </div>
  );
};
