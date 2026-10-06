import React from "react";
import { FolderGit2, Plus } from "lucide-react";
import type { Job, Mission, ProjectSummary } from "../../types/api";
import { bridge } from "../../api/bridge";
import { projectStatus } from "../../utils/projectStatus";
import { Page, PageHeader } from "../../components/page/Page";
import { Dot, PrimaryButton } from "../../components/page/kit";

interface Props {
  projects: ProjectSummary[];
  jobs: Job[];
  onOpenProject: (name: string) => void;
  onAddProject?: () => void;
}

export const useMissions = (intervalMs = 15_000): Mission[] => {
  const [missions, setMissions] = React.useState<Mission[]>([]);
  React.useEffect(() => {
    let live = true;
    const load = () =>
      bridge
        .listMissions()
        .then((m) => live && setMissions(m))
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, intervalMs);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [intervalMs]);
  return missions;
};

export const ProjectsHomeView: React.FC<Props> = ({ projects, jobs, onOpenProject, onAddProject }) => {
  const missions = useMissions();

  return (
    <Page testId="projects-home">
      <PageHeader
        title="Projects"
        subtitle="Each project has its own manager. Open one to see what it is doing."
        actions={
          onAddProject && (
            <PrimaryButton onClick={onAddProject}>
              <Plus className="size-3.5" aria-hidden="true" />
              Add project
            </PrimaryButton>
          )
        }
      />
      <div className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-4">
        {projects.map((project) => {
          const s = projectStatus(project.name, missions, jobs);
          const busy = s.live.length > 0 || s.runningJobs.length > 0;
          return (
            <button
              key={project.name}
              type="button"
              onClick={() => onOpenProject(project.name)}
              className="group flex flex-col gap-4 rounded-[14px] border border-border bg-card p-4 text-left shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] transition-colors hover:border-primary/40 hover:bg-secondary/40"
            >
              <div className="flex items-center gap-2.5">
                <span className="flex size-8 items-center justify-center rounded-lg bg-muted text-success">
                  <FolderGit2 className="size-4" aria-hidden="true" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[14px] font-semibold text-foreground">{project.name}</div>
                  <div className="truncate font-mono text-[11px] text-muted-foreground">
                    {project.repos[0]?.split("/").slice(-1)[0] ?? "no repo"}
                  </div>
                </div>
                <span className={busy ? "text-success" : "text-muted-foreground"}>
                  <Dot live={busy} />
                </span>
              </div>

              <p className="m-0 min-h-[34px] text-[12.5px] leading-snug text-muted-foreground">{s.headline}</p>

              <div className="flex flex-col gap-1.5">
                <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full bg-gradient-to-b from-[#19e0a5] to-[#00b582] transition-[width]"
                    style={{ width: `${s.progress}%` }}
                  />
                </div>
                <div className="flex justify-between font-mono text-[10.5px] text-muted-foreground">
                  <span>
                    {s.milestonesTotal ? `${s.milestonesPassed}/${s.milestonesTotal} milestones` : "no goals yet"}
                  </span>
                  <span>{s.progress}%</span>
                </div>
              </div>

              <div className="flex gap-4 font-mono text-[11px] text-muted-foreground">
                <span>{s.live.length} missions</span>
                <span>{s.runningJobs.length} tasks</span>
                {s.attention.length > 0 && <span className="text-warning">{s.attention.length} to look at</span>}
              </div>
            </button>
          );
        })}
      </div>
    </Page>
  );
};
