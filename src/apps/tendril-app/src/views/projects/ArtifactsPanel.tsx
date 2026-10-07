import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import type { EvidenceGroup } from "../../types/projectAssets";
import { bridge } from "../../api/bridge";
import { Pill } from "../../components/page/kit";
import { Tile, Viewer, when, type FlatItem } from "./EvidenceViewer";

type Filter = "all" | "image" | "video";

/**
 * The project's artifacts: every screenshot and recording its workers attached to any plan, newest
 * first, grouped by the mission and milestone (or plan) they were recorded for. This is where "does it
 * actually work" gets answered at a glance, without opening a mission.
 */
export const ArtifactsPanel: React.FC<{ project: string; busy: boolean; onOpenMission: (id: string) => void }> = ({
  project,
  busy,
  onOpenMission,
}) => {
  const [groups, setGroups] = React.useState<EvidenceGroup[] | null>(null);
  const [filter, setFilter] = React.useState<Filter>("all");
  const [open, setOpen] = React.useState<number | null>(null);

  React.useEffect(() => {
    let live = true;
    setGroups(null);
    const load = () =>
      bridge
        .getProjectEvidence(project)
        .then((e) => live && setGroups(e.groups))
        .catch(() => live && setGroups((g) => g ?? []));
    void load();
    // New evidence arrives while the manager's workers run.
    const timer = busy ? window.setInterval(load, 15_000) : undefined;
    return () => {
      live = false;
      if (timer) window.clearInterval(timer);
    };
  }, [project, busy]);

  const visible = React.useMemo(
    () =>
      (groups ?? [])
        .map((g) => ({ ...g, items: g.items.filter((i) => filter === "all" || i.kind === filter) }))
        .filter((g) => g.items.length > 0),
    [groups, filter],
  );
  const flat = React.useMemo<FlatItem[]>(
    () => visible.flatMap((g) => g.items.map((i) => ({ ...i, group: g.missionTitle ? `${g.missionTitle} · ${g.title}` : g.title }))),
    [visible],
  );
  const counts = React.useMemo(() => {
    const all = (groups ?? []).flatMap((g) => g.items);
    return { all: all.length, image: all.filter((i) => i.kind === "image").length, video: all.filter((i) => i.kind === "video").length };
  }, [groups]);

  if (groups === null) return <div className="p-5 text-[13px] text-muted-foreground">Loading artifacts…</div>;

  if (counts.all === 0) {
    return (
      <div className="m-5 rounded-xl border border-dashed border-border p-8 text-center text-[13px] text-muted-foreground">
        No screenshots or recordings yet. When workers change something you can see, they attach proof here.
      </div>
    );
  }

  const filters: { value: Filter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "image", label: "Screenshots" },
    { value: "video", label: "Recordings" },
  ];

  return (
    <div className="flex flex-col gap-5 p-5" data-testid="artifacts-panel">
      <div className="flex flex-wrap items-center gap-2">
        {filters.map((f) => (
          <button
            key={f.value}
            type="button"
            onClick={() => setFilter(f.value)}
            aria-pressed={filter === f.value}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-[12px] transition-colors",
              filter === f.value
                ? "border-primary/50 bg-primary/10 text-foreground"
                : "border-border text-muted-foreground hover:text-foreground",
            )}
          >
            {f.label}
            <span className="font-mono text-[10.5px] text-muted-foreground">{counts[f.value]}</span>
          </button>
        ))}
      </div>

      {visible.map((g) => (
        <section key={g.planFolder} className="flex flex-col gap-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[13px] font-medium text-foreground">{g.title}</span>
            {g.missionTitle && (
              <button
                type="button"
                onClick={() => g.missionId && onOpenMission(g.missionId)}
                className="rounded-full border border-border px-2 py-0.5 font-mono text-[10.5px] text-muted-foreground hover:text-foreground"
                title="Open the mission"
              >
                {g.missionTitle}
              </button>
            )}
            {g.newest && <span className="font-mono text-[10.5px] text-muted-foreground">{when(g.newest)}</span>}
            <Pill>{g.items.length}</Pill>
          </div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-3">
            {g.items.map((item) => (
              <Tile key={item.path} item={item} onOpen={() => setOpen(flat.findIndex((f) => f.path === item.path))} />
            ))}
          </div>
        </section>
      ))}

      {open !== null && flat[open] && (
        <Viewer items={flat} index={open} onIndex={setOpen} onClose={() => setOpen(null)} />
      )}
    </div>
  );
};
