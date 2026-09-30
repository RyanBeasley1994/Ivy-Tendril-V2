import React from "react";
import { createPortal } from "react-dom";
import {
  Activity,
  ArrowRight,
  ChartColumn,
  CircleX,
  Clock,
  Eye,
  FileText,
  Inbox,
  Keyboard,
  LayoutGrid,
  MessageCircle,
  OctagonPause,
  Plus,
  Search,
  Settings,
  Snowflake,
  GitPullRequest,
  ThumbsUp,
} from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import { useEnumLabels } from "../../i18n/enumLabels";
import type { Job, PlanSummary } from "../../types/api";
import { AgentAvatar, Kbd, Label, Pill, modKey, type Tone } from "../../components/page/kit";
import { buildDecisions } from "../../utils/fleet";
import { ACTIVE_JOB_STATUSES } from "../../utils/processStatus";
import { formatAge } from "../../utils/commandCenter";

type Icon = React.ComponentType<{ className?: string }>;

interface PaletteItem {
  id: string;
  group: "actions" | "attention" | "plans" | "jobs" | "goto";
  icon: Icon;
  tile: string;
  title: React.ReactNode;
  /** What the query is matched against. */
  text: string;
  sub?: string;
  right?: React.ReactNode;
  run: () => void;
}

const STATE_TONE: Record<string, Tone> = {
  Draft: "mute",
  Executing: "ok",
  Review: "info",
  Failed: "bad",
  Completed: "violet",
  Blocked: "warn",
  Creating: "ok",
  Updating: "ok",
};

const MAX_PLANS = 6;

/** Every whitespace-separated term appears somewhere in the text. */
const matches = (query: string, text: string): boolean => {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay = text.toLowerCase();
  return q.split(/\s+/).every((term) => hay.includes(term));
};

/**
 * The ⌘K palette: actions, what needs you, plans and live jobs, and every page, over one query. The
 * database-wide plan search it replaces is its last row, so nothing that was reachable is lost.
 */
export const CommandPalette: React.FC<{
  plans: PlanSummary[];
  jobs: Job[];
  onClose: () => void;
  onOpenPlan: (planId: string) => void;
  onOpenJob: (jobId: string) => void;
  onNavigate: (navId: string) => void;
  onNewPlan: (description?: string) => void;
  onNewChat: () => void;
  onStopAll: () => void;
  onShowShortcuts: () => void;
  onSearchAllPlans: () => void;
}> = ({
  plans,
  jobs,
  onClose,
  onOpenPlan,
  onOpenJob,
  onNavigate,
  onNewPlan,
  onNewChat,
  onStopAll,
  onShowShortcuts,
  onSearchAllPlans,
}) => {
  const { t } = useTranslation("common");
  const labels = useEnumLabels();
  const [query, setQuery] = React.useState("");
  const [selected, setSelected] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);
  const now = Date.now();

  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const close = (fn: () => void) => () => {
    onClose();
    fn();
  };

  const neutral = "bg-secondary text-muted-foreground";
  const q = query.trim();
  const activeJobs = jobs.filter((j) => ACTIVE_JOB_STATUSES.includes(j.status));

  const actionItems: PaletteItem[] = [
    {
      id: "new-plan",
      group: "actions" as const,
      icon: Plus,
      tile: "bg-primary/12 text-success",
      title: q ? t("palette.newPlanFrom", { text: q }) : t("palette.newPlan"),
      text: `${t("palette.newPlan")} ${q}`,
      right: (
        <>
          <Kbd>{modKey()}</Kbd>
          <Kbd>N</Kbd>
        </>
      ),
      run: close(() => onNewPlan(q || undefined)),
    },
    {
      id: "new-chat",
      group: "actions" as const,
      icon: MessageCircle,
      tile: neutral,
      title: t("palette.newChat"),
      text: t("palette.newChat"),
      run: close(onNewChat),
    },
    ...(activeJobs.length > 0
      ? [
          {
            id: "stop-all",
            group: "actions" as const,
            icon: OctagonPause,
            tile: "bg-destructive/14 text-destructive",
            title: t("palette.stopAll"),
            text: t("palette.stopAll"),
            sub: t("palette.jobsActive", { count: activeJobs.length }),
            run: close(onStopAll),
          },
        ]
      : []),
    {
      id: "shortcuts",
      group: "actions" as const,
      icon: Keyboard,
      tile: neutral,
      title: t("palette.shortcuts"),
      text: t("palette.shortcuts"),
      right: <Kbd>?</Kbd>,
      run: close(onShowShortcuts),
    },
  ];
  const actions = actionItems.filter((item) => matches(q, item.text) || item.id === "new-plan");

  const decisions = buildDecisions(plans, jobs, now).items;
  const attention: PaletteItem[] = decisions
    .filter((d) => d.kind !== "draft")
    .map((d) => {
      const icon = d.kind === "review" ? Eye : d.kind === "failed" ? CircleX : Clock;
      const tile =
        d.kind === "review"
          ? "bg-info/14 text-info"
          : d.kind === "failed"
            ? "bg-destructive/14 text-destructive"
            : "bg-warning/13 text-warning";
      const target = d.target;
      return {
        id: `att:${target}:${d.kind}`,
        group: "attention" as const,
        icon,
        tile,
        title: d.title,
        text: `${d.title} ${d.planId ?? ""} ${d.kind}`,
        sub: [d.planId ? `#${d.planId}` : null, t(`palette.attention.${d.kind}` as "palette.attention.review"), d.waitingMs != null ? formatAge(d.waitingMs) : null]
          .filter(Boolean)
          .join(" · "),
        run: close(() =>
          target.startsWith("plan:") ? onOpenPlan(target.slice(5)) : onOpenJob(target.slice(4)),
        ),
      };
    })
    .filter((item) => matches(q, item.text));

  const planItems: PaletteItem[] = (q ? plans : [])
    .filter((p) => matches(q, `${p.title} ${p.id} #${p.id} ${p.project} ${p.state}`))
    .sort((a, b) => (Date.parse(b.updated ?? "") || 0) - (Date.parse(a.updated ?? "") || 0))
    .slice(0, MAX_PLANS)
    .map((p) => ({
      id: `plan:${p.id}`,
      group: "plans" as const,
      icon: FileText,
      tile: neutral,
      title: p.title,
      text: p.title,
      sub: `#${p.id} · ${p.project}`,
      right: <Pill tone={STATE_TONE[p.state] ?? "mute"}>{labels.planState(p.state)}</Pill>,
      run: close(() => onOpenPlan(p.id)),
    }));

  const jobItems: PaletteItem[] = activeJobs
    .filter((j) => matches(q, `${j.planTitle ?? ""} ${j.planId ?? ""} ${j.type} ${j.project}`))
    .slice(0, 5)
    .map((j) => ({
      id: `job:${j.id}`,
      group: "jobs" as const,
      icon: Activity,
      tile: "bg-primary/12 text-success",
      title: j.planTitle || labels.jobType(j.type),
      text: j.planTitle ?? j.type,
      sub: [j.planId ? `#${j.planId}` : null, labels.jobType(j.type), labels.jobStatus(j.status)].filter(Boolean).join(" · "),
      right: <AgentAvatar agent={j.model} size={18} />,
      run: close(() => onOpenJob(j.id)),
    }));

  const pages: [string, Icon, string][] = [
    ["dashboard", LayoutGrid, t("palette.pages.dashboard")],
    ["plans", FileText, t("sidebar.nav.plans")],
    ["review", ThumbsUp, t("sidebar.nav.review")],
    ["jobs", Activity, t("sidebar.nav.jobs")],
    ["insights", ChartColumn, t("sidebar.nav.insights")],
    ["pull-requests", GitPullRequest, t("appTitles.pullRequests")],
    ["inbox", Inbox, t("appTitles.inbox")],
    ["icebox", Snowflake, t("appTitles.icebox")],
    ["settings", Settings, t("appTitles.settings")],
  ];
  const goto: PaletteItem[] = pages
    .filter(([, , label]) => matches(q, label))
    .slice(0, q ? 9 : 4)
    .map(([nav, icon, label]) => ({
      id: `goto:${nav}`,
      group: "goto" as const,
      icon,
      tile: neutral,
      title: label,
      text: label,
      right: <ArrowRight className="size-3.5 text-muted-foreground" />,
      run: close(() => onNavigate(nav)),
    }));

  const searchAll: PaletteItem = {
    id: "search-all",
    group: "plans",
    icon: Search,
    tile: neutral,
    title: q ? t("palette.searchAllFor", { text: q }) : t("palette.searchAll"),
    text: "search",
    sub: t("palette.searchAllHint"),
    run: close(onSearchAllPlans),
  };

  const allGroups: { key: PaletteItem["group"]; label: string; items: PaletteItem[] }[] = [
    { key: "actions", label: t("palette.groups.actions"), items: actions },
    { key: "attention", label: t("palette.groups.attention"), items: attention },
    { key: "jobs", label: t("palette.groups.jobs"), items: jobItems },
    { key: "plans", label: t("palette.groups.plans"), items: [...planItems, searchAll] },
    { key: "goto", label: t("palette.groups.goto"), items: goto },
  ];
  const groups = allGroups.filter((g) => g.items.length > 0);

  const flat = groups.flatMap((g) => g.items);
  const current = Math.min(selected, Math.max(0, flat.length - 1));

  React.useEffect(() => setSelected(0), [query]);
  React.useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${current}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [current]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((i) => (i + 1) % Math.max(1, flat.length));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((i) => (i - 1 + flat.length) % Math.max(1, flat.length));
    } else if (e.key === "Enter") {
      e.preventDefault();
      flat[current]?.run();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  let index = -1;
  return createPortal(
    <div
      className="fixed inset-0 z-[1000] bg-[rgba(4,6,9,0.66)] backdrop-blur-[3px]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      data-testid="command-palette"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("palette.title")}
        onKeyDown={onKeyDown}
        className="absolute left-1/2 top-[12vh] flex max-h-[72vh] w-[680px] max-w-[calc(100vw-32px)] -translate-x-1/2 flex-col overflow-hidden rounded-2xl border border-input bg-card shadow-[inset_0_0_0_1px_rgba(255,255,255,0.03),0_40px_120px_-20px_rgba(0,0,0,0.9),0_0_0_8px_rgba(0,204,146,0.04)]"
      >
        <div className="flex h-[60px] shrink-0 items-center gap-3 border-b border-border px-[18px]">
          <Search className="size-[17px] text-muted-foreground" aria-hidden="true" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("palette.placeholder")}
            aria-label={t("palette.placeholder")}
            role="combobox"
            aria-expanded="true"
            aria-controls="command-palette-list"
            aria-activedescendant={flat[current] ? `cp-${current}` : undefined}
            className="min-w-0 flex-1 border-0 bg-transparent text-base text-foreground outline-none placeholder:text-muted-foreground"
          />
          <Kbd>esc</Kbd>
        </div>

        <div ref={listRef} id="command-palette-list" role="listbox" className="min-h-0 flex-1 overflow-y-auto p-2">
          {groups.map((group) => (
            <div key={group.key} className="flex flex-col gap-0.5 pb-1">
              <Label className="px-3 pb-1.5 pt-2">{group.label}</Label>
              {group.items.map((item) => {
                index += 1;
                const i = index;
                const Icon = item.icon;
                const isSel = i === current;
                return (
                  <div
                    key={item.id}
                    id={`cp-${i}`}
                    data-index={i}
                    role="option"
                    aria-selected={isSel}
                    onMouseMove={() => setSelected(i)}
                    onClick={item.run}
                    className={cn(
                      "flex h-11 cursor-pointer items-center gap-3 rounded-[9px] px-3",
                      isSel && "bg-primary/[0.08] shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--primary)_28%,transparent)]",
                    )}
                  >
                    <span className={cn("flex size-7 shrink-0 items-center justify-center rounded-lg", item.tile)}>
                      <Icon className="size-3.5" />
                    </span>
                    <span className="flex min-w-0 flex-col gap-0.5">
                      <span className="truncate text-[13px]">{item.title}</span>
                      {item.sub && <span className="truncate text-[11.5px] text-muted-foreground">{item.sub}</span>}
                    </span>
                    <span className="ml-auto flex shrink-0 items-center gap-1.5">
                      {item.right}
                      {isSel && !item.right && <Kbd>⏎</Kbd>}
                    </span>
                  </div>
                );
              })}
            </div>
          ))}
        </div>

        <div className="flex h-10 shrink-0 items-center gap-4 border-t border-border bg-background/60 px-[18px] font-mono text-[10.5px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd>
            {t("palette.keys.navigate")}
          </span>
          <span className="flex items-center gap-1">
            <Kbd>⏎</Kbd>
            {t("palette.keys.run")}
          </span>
          <span className="ml-auto flex items-center gap-1">
            <Kbd>esc</Kbd>
            {t("palette.keys.close")}
          </span>
        </div>
      </div>
    </div>,
    document.body,
  );
};
