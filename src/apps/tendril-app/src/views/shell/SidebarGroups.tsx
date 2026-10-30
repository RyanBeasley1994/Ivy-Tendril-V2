import React from "react";
import { ChevronRight, FolderPlus, MoreHorizontal, Plus, Search, X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@ivy-interactive/components/ui";
import type { Job, ProjectSummary } from "../../types/api";
import { bridge } from "../../api/bridge";
import { projectStatus } from "../../utils/projectStatus";
import { useMissions } from "../projects/ProjectsHomeView";
import { useManagerActivity } from "../../state/managerActivity";
import { useManagerUnread } from "../../state/managerUnread";
import { usePointerDrag } from "../../hooks/usePointerDrag";

/**
 * The sidebar's project groups: the operator's own folders of projects, for getting to the ones they
 * work on in a click. A project shows a dot while its manager or its work is running, and turns amber
 * with a count when its manager needs the operator. Projects in no group sit under "Other projects".
 *
 * Saved per device in the app's UI state (`ui_state.json` on desktop, local storage in a browser).
 */

interface Group {
  id: string;
  name: string;
  projects: string[];
  collapsed?: boolean;
  /** One of {@link GROUP_COLORS}; none when unset. */
  color?: string;
}

/** What a group can be coloured, from the app's palette. */
const GROUP_COLORS: { name: string; value: string }[] = [
  { name: "Blue", value: "#1b92ff" },
  { name: "Teal", value: "#2dd4bf" },
  { name: "Green", value: "#4ade9d" },
  { name: "Amber", value: "#fbbf24" },
  { name: "Orange", value: "#fb923c" },
  { name: "Red", value: "#ff8a8a" },
  { name: "Pink", value: "#f472b6" },
  { name: "Violet", value: "#a78bfa" },
];

/** The menu components a group's options render through: the ⋯ dropdown and the right-click menu. */
interface MenuKit {
  Item: React.ElementType;
  Sub: React.ElementType;
  SubTrigger: React.ElementType;
  SubContent: React.ElementType;
  Separator: React.ElementType;
}
const DROPDOWN: MenuKit = {
  Item: DropdownMenuItem,
  Sub: DropdownMenuSub,
  SubTrigger: DropdownMenuSubTrigger,
  SubContent: DropdownMenuSubContent,
  Separator: DropdownMenuSeparator,
};
const CONTEXT: MenuKit = {
  Item: ContextMenuItem,
  Sub: ContextMenuSub,
  SubTrigger: ContextMenuSubTrigger,
  SubContent: ContextMenuSubContent,
  Separator: ContextMenuSeparator,
};

const STATE_KEY = "projectGroups";
const OTHER_ID = "__other";

const load = async (): Promise<{ groups: Group[]; otherCollapsed: boolean }> => {
  try {
    const raw = await bridge.loadUiState(STATE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as { groups?: Group[]; otherCollapsed?: boolean };
      return { groups: parsed.groups ?? [], otherCollapsed: parsed.otherCollapsed ?? true };
    }
  } catch {
    // Nothing saved yet, or unreadable: start empty.
  }
  return { groups: [], otherCollapsed: true };
};

const newId = () => Math.random().toString(36).slice(2, 10);

type Activity = { busy: boolean; needsYou: number; unread: boolean };

interface Props {
  projects: ProjectSummary[];
  jobs: Job[];
  activeProject: string | null;
  onOpenProject: (name: string) => void;
  /** Opens the search palette (⌘K). */
  onSearch?: () => void;
  /** Opens the Add Project dialog. */
  onAddProject?: () => void;
}

/** What is being dragged: a project out of a group (or "Other projects"), or a whole group. */
type Dragged = { kind: "project"; project: string; from: string } | { kind: "group"; id: string };

/** Drop targets: a group (header or empty space: append) or a project row (insert before it). */
const groupTarget = (groupId: string) => `g|${groupId}`;
const rowTarget = (groupId: string, project: string) => `r|${groupId}|${project}`;
const parseTarget = (t: string): { groupId: string; before?: string } | null => {
  const [kind, groupId, ...rest] = t.split("|");
  if (!groupId) return null;
  return kind === "r" ? { groupId, before: rest.join("|") } : { groupId };
};

export const SidebarGroups: React.FC<Props> = ({ projects, jobs, activeProject, onOpenProject, onSearch, onAddProject }) => {
  const [groups, setGroups] = React.useState<Group[] | null>(null);
  const [otherCollapsed, setOtherCollapsed] = React.useState(true);
  const [editing, setEditing] = React.useState<string | null>(null);
  const missions = useMissions(10_000);
  const activity = useManagerActivity();
  const unread = useManagerUnread(activity, activeProject);

  React.useEffect(() => {
    void load().then((s) => {
      setGroups(s.groups);
      setOtherCollapsed(s.otherCollapsed);
    });
  }, []);

  const save = (next: Group[], nextOther = otherCollapsed) => {
    setGroups(next);
    setOtherCollapsed(nextOther);
    void bridge.saveUiState(STATE_KEY, JSON.stringify({ groups: next, otherCollapsed: nextOther })).catch(() => undefined);
  };

  const status = React.useMemo(() => {
    const out: Record<string, Activity> = {};
    for (const p of projects) {
      const a = activity[p.name];
      const s = projectStatus(p.name, missions, jobs, a?.working ?? false);
      out[p.name] = {
        busy: s.busy,
        needsYou: (a?.needsYou ?? 0) + (s.attention.length > 0 && !a?.needsYou ? 1 : 0),
        unread: unread.has(p.name),
      };
    }
    return out;
  }, [projects, missions, jobs, activity, unread]);

  // A project dropped on a row goes just above it (reordering within a group, or moving between
  // groups at that spot); on a group it goes to the end; on "Other projects" it leaves every group.
  // A group dropped on another goes above it; on "Other projects", to the bottom.
  const dnd = usePointerDrag<Dragged>((d, target) => {
    const t = parseTarget(target);
    if (!groups || !t) return;
    if (d.kind === "group") {
      if (t.groupId === d.id) return;
      const moving = groups.find((g) => g.id === d.id);
      if (!moving) return;
      const rest = groups.filter((g) => g.id !== d.id);
      const at = t.groupId === OTHER_ID ? rest.length : rest.findIndex((g) => g.id === t.groupId);
      save([...rest.slice(0, at < 0 ? rest.length : at), moving, ...rest.slice(at < 0 ? rest.length : at)]);
      return;
    }
    if (t.before === d.project) return;
    if (t.groupId === OTHER_ID) {
      save(groups.map((g) => ({ ...g, projects: g.projects.filter((p) => p !== d.project) })));
      return;
    }
    save(
      groups.map((g) => {
        let list = g.projects;
        if (g.id === d.from || g.id === t.groupId) list = list.filter((p) => p !== d.project);
        if (g.id !== t.groupId) return list === g.projects ? g : { ...g, projects: list };
        const at = t.before ? list.indexOf(t.before) : -1;
        return { ...g, projects: at < 0 ? [...list, d.project] : [...list.slice(0, at), d.project, ...list.slice(at)] };
      }),
    );
  });
  const dropTarget = dnd.over;

  if (groups === null) return null;

  const names = new Set(projects.map((p) => p.name));
  const grouped = new Set(groups.flatMap((g) => g.projects));
  const other = projects.map((p) => p.name).filter((n) => !grouped.has(n));

  const update = (id: string, f: (g: Group) => Group | null) =>
    save(groups.flatMap((g) => (g.id === id ? [f(g)].filter((x): x is Group => x !== null) : [g])));
  const addTo = (groupId: string, project: string) =>
    save(
      groups.map((g) =>
        g.id === groupId
          ? { ...g, projects: g.projects.includes(project) ? g.projects : [...g.projects, project] }
          : g,
      ),
    );
  const removeFrom = (groupId: string, project: string) =>
    update(groupId, (g) => ({ ...g, projects: g.projects.filter((p) => p !== project) }));
  const createGroup = () => {
    const g: Group = { id: newId(), name: "New group", projects: [] };
    save([...groups, g]);
    setEditing(g.id);
  };

  const row = (project: string, groupId: string, color?: string) => {
    const s = status[project] ?? { busy: false, needsYou: 0, unread: false };
    return (
      <div
        key={project}
        {...dnd.target(rowTarget(groupId, project))}
        className={cn(
          "group/row relative rounded-md",
          dropTarget === rowTarget(groupId, project) && "shadow-[inset_0_2px_0_var(--primary)]",
        )}
      >
        {color && (
          <span className="absolute bottom-1 left-[13px] top-1 w-0.5 rounded-full opacity-50" style={{ background: color }} aria-hidden="true" />
        )}
        <button
          type="button"
          {...dnd.source({ kind: "project", project, from: groupId }, project)}
          data-drawer-close
          onClick={() => onOpenProject(project)}
          aria-current={activeProject === project ? "page" : undefined}
          className={cn(
            "flex h-8 w-full items-center gap-2 rounded-md pl-7 pr-2 text-left text-[13px] transition-colors",
            activeProject === project
              ? "bg-secondary text-foreground"
              : s.unread
                ? "bg-primary/12 font-medium text-foreground shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--primary)_35%,transparent)] hover:bg-primary/18"
                : "text-muted-foreground hover:bg-secondary/60 hover:text-foreground",
          )}
          title={s.unread ? "The manager has replied" : undefined}
        >
          <ActivityDot busy={s.busy} needsYou={s.needsYou} unread={s.unread} />
          <span className="min-w-0 flex-1 truncate">{project}</span>
          {s.needsYou > 0 && (
            <span className="rounded-full bg-warning/15 px-1.5 font-mono text-[10px] text-warning">{s.needsYou}</span>
          )}
        </button>
        {groupId !== OTHER_ID && (
          <button
            type="button"
            onClick={() => removeFrom(groupId, project)}
            aria-label={`Remove ${project} from group`}
            className="absolute right-1.5 top-1/2 hidden size-5 -translate-y-1/2 items-center justify-center rounded text-muted-foreground hover:text-foreground group-hover/row:inline-flex"
          >
            <X className="size-3" aria-hidden="true" />
          </button>
        )}
      </div>
    );
  };

  const header = (
    id: string,
    label: React.ReactNode,
    collapsed: boolean,
    toggle: () => void,
    count: { busy: number; needs: number; unread: number },
    menu?: React.ReactNode,
    color?: string,
  ) => (
    <div
      {...dnd.target(groupTarget(id))}
      className={cn(
        "group/head flex h-8 items-center gap-1 rounded-md px-1",
        dropTarget === groupTarget(id) &&
          (dnd.dragging?.kind === "group" ? "shadow-[inset_0_2px_0_var(--primary)]" : "bg-primary/10 ring-1 ring-primary/40"),
      )}
    >
      <button
        type="button"
        {...(id !== OTHER_ID ? dnd.source({ kind: "group", id }, typeof label === "string" ? label : "Group") : {})}
        onClick={toggle}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-[12.5px] font-semibold text-muted-foreground hover:text-foreground"
      >
        <ChevronRight className={cn("size-3.5 shrink-0 transition-transform", !collapsed && "rotate-90")} aria-hidden="true" />
        {color && <span className="size-2.5 shrink-0 rounded-[3px]" style={{ background: color }} aria-hidden="true" />}
        <span className="min-w-0 truncate" style={color ? { color } : undefined}>{label}</span>
        {collapsed && (count.busy > 0 || count.needs > 0 || count.unread > 0) && (
          <ActivityDot busy={count.busy > 0} needsYou={count.needs} unread={count.unread > 0} className="ml-1" />
        )}
      </button>
      {menu}
    </div>
  );

  /** A group's options, through either menu: add a project, rename, colour, delete. */
  const groupMenu = (g: Group, candidates: string[], k: MenuKit) => (
    <>
      <k.Sub>
        <k.SubTrigger disabled={candidates.length === 0}>Add project</k.SubTrigger>
        <k.SubContent className="max-h-72 overflow-y-auto">
          {candidates.map((n) => (
            <k.Item key={n} onSelect={() => addTo(g.id, n)}>{n}</k.Item>
          ))}
        </k.SubContent>
      </k.Sub>
      <k.Item onSelect={() => setEditing(g.id)}>Rename</k.Item>
      <k.Sub>
        <k.SubTrigger>
          <span className="mr-2 size-2.5 rounded-[3px] border border-border" style={g.color ? { background: g.color, borderColor: g.color } : undefined} aria-hidden="true" />
          Colour
        </k.SubTrigger>
        <k.SubContent>
          {GROUP_COLORS.map((c) => (
            <k.Item key={c.value} onSelect={() => update(g.id, (x) => ({ ...x, color: c.value }))}>
              <span className="mr-2 size-3 rounded-[3px]" style={{ background: c.value }} aria-hidden="true" />
              {c.name}
              {g.color === c.value && <span className="ml-auto pl-3 text-muted-foreground">✓</span>}
            </k.Item>
          ))}
          <k.Separator />
          <k.Item onSelect={() => update(g.id, (x) => ({ ...x, color: undefined }))}>No colour</k.Item>
        </k.SubContent>
      </k.Sub>
      <k.Separator />
      <k.Item onSelect={() => update(g.id, () => null)}>Delete group</k.Item>
    </>
  );

  const counts = (list: string[]) => ({
    busy: list.filter((p) => status[p]?.busy).length,
    needs: list.reduce((n, p) => n + (status[p]?.needsYou ?? 0), 0),
    unread: list.filter((p) => status[p]?.unread).length,
  });

  return (
    <div className="tcc-sidebar-groups mb-2 mt-2 flex min-h-0 shrink flex-col gap-0.5 overflow-y-auto" data-testid="sidebar-groups">
      <div className="flex items-center px-2.5 pb-1">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">Groups</span>
        {onSearch && (
          <button
            type="button"
            onClick={onSearch}
            aria-label="Search"
            title="Search (⌘K)"
            className="ml-auto inline-flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
          >
            <Search className="size-3.5" aria-hidden="true" />
          </button>
        )}
        {onAddProject && (
          <button
            type="button"
            onClick={onAddProject}
            aria-label="New project"
            title="New project"
            className={cn(
              "inline-flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground",
              !onSearch && "ml-auto",
            )}
          >
            <FolderPlus className="size-3.5" aria-hidden="true" />
          </button>
        )}
        <button
          type="button"
          onClick={createGroup}
          aria-label="New group"
          title="New group"
          className={cn(
            "inline-flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground",
            !onSearch && !onAddProject && "ml-auto",
          )}
        >
          <Plus className="size-3.5" aria-hidden="true" />
        </button>
      </div>

      {groups.length === 0 && (
        <button
          type="button"
          onClick={createGroup}
          className="mx-1 rounded-md border border-dashed border-border px-2 py-2 text-left text-[11.5px] text-muted-foreground hover:text-foreground"
        >
          Create a group, then drag projects into it.
        </button>
      )}

      {groups.map((g) => {
        const present = g.projects.filter((p) => names.has(p));
        const candidates = projects.map((p) => p.name).filter((n) => !g.projects.includes(n));
        return (
          <div key={g.id} className="flex flex-col">
            {editing === g.id ? (
              <input
                autoFocus
                defaultValue={g.name}
                onFocus={(e) => e.currentTarget.select()}
                onBlur={(e) => {
                  update(g.id, (x) => ({ ...x, name: e.currentTarget.value.trim() || x.name }));
                  setEditing(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.currentTarget.blur();
                  if (e.key === "Escape") setEditing(null);
                }}
                className="mx-1 h-8 rounded-md border border-primary/50 bg-muted px-2 text-[12.5px] text-foreground outline-none"
              />
            ) : (
              <ContextMenu>
                <ContextMenuTrigger asChild>
                  <div>
                    {header(
                      g.id,
                      g.name,
                      !!g.collapsed,
                      () => update(g.id, (x) => ({ ...x, collapsed: !x.collapsed })),
                      counts(present),
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            aria-label={`${g.name} options`}
                            className="inline-flex size-6 items-center justify-center rounded text-muted-foreground opacity-0 hover:bg-secondary hover:text-foreground focus:opacity-100 group-hover/head:opacity-100"
                          >
                            <MoreHorizontal className="size-4" aria-hidden="true" />
                          </button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">{groupMenu(g, candidates, DROPDOWN)}</DropdownMenuContent>
                      </DropdownMenu>,
                      g.color,
                    )}
                  </div>
                </ContextMenuTrigger>
                <ContextMenuContent>{groupMenu(g, candidates, CONTEXT)}</ContextMenuContent>
              </ContextMenu>
            )}
            {!g.collapsed && (
              <div {...dnd.target(groupTarget(g.id))} className={cn("flex flex-col rounded-md", dropTarget === groupTarget(g.id) && "bg-primary/5")}>
                {present.length === 0 && (
                  <span className="pl-7 text-[11.5px] leading-8 text-muted-foreground/70">Drag projects here</span>
                )}
                {present.map((p) => row(p, g.id, g.color))}
              </div>
            )}
          </div>
        );
      })}

      {dnd.ghost}
      {other.length > 0 && (
        <div className="flex flex-col">
          {header(
            OTHER_ID,
            `Other projects · ${other.length}`,
            otherCollapsed,
            () => save(groups, !otherCollapsed),
            counts(other),
          )}
          {!otherCollapsed && <div className="flex flex-col">{other.map((p) => row(p, OTHER_ID))}</div>}
        </div>
      )}
    </div>
  );
};

/** Green and pulsing while something runs; amber when the manager needs the operator; nothing when idle. */
const ActivityDot: React.FC<{ busy: boolean; needsYou: number; unread?: boolean; className?: string }> = ({
  busy,
  needsYou,
  unread,
  className,
}) => {
  if (needsYou > 0) return <span className={cn("size-1.5 shrink-0 rounded-full bg-warning", className)} aria-label="Needs you" />;
  if (busy)
    return (
      <span className={cn("relative size-1.5 shrink-0", className)} aria-label="Working">
        <span className="absolute inset-0 animate-ping rounded-full bg-success/60" />
        <span className="absolute inset-0 rounded-full bg-success" />
      </span>
    );
  if (unread) return <span className={cn("size-1.5 shrink-0 rounded-full bg-primary", className)} aria-label="Unread reply" />;
  return <span className={cn("size-1.5 shrink-0", className)} aria-hidden="true" />;
};
