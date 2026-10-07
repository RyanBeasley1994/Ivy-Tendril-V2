import React from "react";
import { ForgeLogo } from "../components/ForgeLogo";
import { MobileTabBar } from "./shell/MobileTabBar";
import { Plus } from "lucide-react";
import { PRODUCT_NAME } from "../branding";
import {
  TendrilShell,
  ShellSidebarHeader,
  ShellNav,
  ShellSidebarSection,
  ShellTabs,
  ShellNewPlanButton,
  ShellSettingsButton,
  type ShellNavItemDto,
  type ShellTabDto,
} from "@ivy-interactive/components/tendril";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@ivy-interactive/components/ui";
import { Construction, GitPullRequest, Snowflake, Info } from "lucide-react";
import type { Job, ProjectSummary, ServiceInfo } from "../types/api";
import { SidebarManagers } from "./shell/SidebarManagers";
import { OfflineBanner } from "../components/OfflineBanner";
// Lazy for the same budget, behind a placeholder of its own height so the page never shifts when
// it arrives.
const ShellTopBar = React.lazy(() =>
  import("./shell/ShellTopBar").then((m) => ({ default: m.ShellTopBar })),
);
// Lazy: the usage card reads the agent's provider through the settings modules, which the shell's
// eager chunk has no other reason to carry (`code-splitting.test.tsx` holds it to a budget).
const SidebarStatus = React.lazy(() =>
  import("./shell/SidebarStatus").then((m) => ({ default: m.SidebarStatus })),
);
import { ServiceStatusBanner } from "../components/service";
import { firstStringArg } from "../utils/eventArgs";
import {
  pageTabTitle,
  usesSidebarList,
  withNavSelection,
  type ShellSidebarList,
} from "../state/sidebarListStore";
import { AGENT_APP_ID, appDescriptor, isFullBleedApp, type SessionPane } from "../state/navigation";
import { i18n, useTranslation, type TFunction } from "../i18n";

/**
 * `common`'s `t` in the language current at each call, for the item builders below when a caller -
 * a test, say - hands them none. The shell passes its own, so a language change re-renders the rows.
 */
const commonT = i18n.getFixedT(null, "common");

/**
 * V1 `TendrilAppShell.PageTabId`. Identifies the strip's leading tab, which reveals the page behind
 * the session panes; V1 gives it a `$` prefix so it cannot collide with a session id.
 */
export const PAGE_TAB_ID = "$page";

/**
 * The shell's content container, and the one place an app's outer padding is decided.
 *
 * V1's host pads every app by 16px and owns its vertical scroll
 * (`Ivy-Framework/.../AppHostWidget.tsx`: `<div className="w-full h-full p-4 overflow-y-auto">`);
 * an app opts out by putting `RemoveParentPadding()` on its root layout, which zeroes the *parent's*
 * padding outright - `padding: 0`, never a reduced padding. {@link CONTENT_PADDED_CLASS} is that
 * default and {@link CONTENT_FULL_BLEED_CLASS} is that opt-out, chosen per page from
 * `AppDescriptor.fullBleed` so views carry no outer padding of their own.
 *
 * Both keep the scroll *inside* the frame rather than on the page. `.tsh-frame-pane` is
 * `position: absolute; inset: 0` with `display: flex; flex-direction: column`, so `flex-1` gives this
 * element a definite height either way - which is what lets a view bound its own scroll viewport
 * (`JobsView`'s `fillHeight` table: a `shrink-0` toolbar over a `flex-1 min-h-0` body) and keep its
 * `position: sticky` header working. A page-level scroll container would break both.
 */
export const CONTENT_PADDED_CLASS = "flex-1 overflow-y-auto px-5 pb-6 pt-4";

/** The full-bleed content container: no padding, and the app owns every scroll inside it. */
export const CONTENT_FULL_BLEED_CLASS = "flex min-h-0 flex-1 flex-col overflow-hidden";

/**
 * The `[App]` icon of the app a nav id names, which is the glyph V1's `$page` tab carries
 * (`BrandedAppDisplay`). Titles come from the router's descriptor table instead of being repeated
 * here, so the strip and the browser title cannot drift from what routing thinks an app is called.
 * A session tab passes no icon at all and gets the terminal glyph.
 */
const pageIcon = (navId: string): string | undefined => {
  switch (navId) {
    case "projects":
      return "FolderGit2";
    case "git":
      return "GitBranch";
    case "dashboard":
      return "ChartBar";
    case "chat":
      return "MessageSquare";
    case "inbox":
      return "Inbox";
    case "plans":
      return "Feather";
    case "review":
      return "ThumbsUp";
    case "recommendations":
      return "Lightbulb";
    case "jobs":
      return "Activity";
    case "missions":
      return "Rocket";
    case "pull-requests":
      return "GitPullRequest";
    case "icebox":
      return "Snowflake";
    case "about":
      return "Info";
    case "settings":
      return "Settings";
    default:
      if (navId.startsWith("project-")) return "FolderGit2";
      if (navId.startsWith("plan-")) return "FileText";
      if (navId.startsWith("job-")) return "Activity";
      return "File";
  }
};

/**
 * The nav's badge counts, keyed by the app id V1 keys them by in
 * `TendrilAppShell.BuildMenuItems`'s `badges` dictionary. Dashboard has no key there, so it never
 * carries one; `icebox`, `chat` and `agent` do, but none of the three is a nav row.
 */
export interface ShellNavBadges {
  plans?: number;
  review?: number;
  recommendations?: number;
  jobs?: number;
}

/**
 * V1 `TendrilAppShell.BuildNavItems`: the sidebar nav is the visible `[App(group: ["Apps"])]` set
 * in `Constants` order - Dashboard 10, Plans 20, Review 30, Recommendations 40, Jobs 50 - minus the
 * entries it drops. Chat (75) and Agent (80) are dropped because the dedicated Chat row above the
 * nav reaches them; Inbox (65) is dropped into the footer by `footerAppIds`; Pull Requests (60),
 * Icebox (70) and Configuration are `isVisible: false` and live in the footer's settings menu. So
 * these five, in this order, are the whole nav.
 *
 * A count of zero shows no badge, as V1's `ShouldShowBadge` requires (`count > 0`).
 */
export const buildNavItems = (
  activeNav: string,
  badges: ShellNavBadges = {},
  t: TFunction<"common"> = commonT,
): ShellNavItemDto[] => {
  const badge = (count: number | undefined) =>
    count !== undefined && count > 0 ? String(count) : undefined;

  const overview = t("sidebar.navGroup.overview");
  const observe = t("sidebar.navGroup.observe");

  // Plans, Review, Missions and Jobs are the manager's to sort, so they are reached from inside a
  // project (and still addressable by URL). Whatever is waiting on a person shows as the Projects badge.
  return [
    {
      id: "projects",
      label: t("sidebar.nav.projects"),
      icon: "FolderGit2",
      badge: badge((badges.plans ?? 0) + (badges.review ?? 0) + (badges.recommendations ?? 0)),
      group: overview,
    },
    { id: "git", label: t("sidebar.nav.git"), icon: "GitBranch", group: overview },
    { id: "dashboard", label: t("sidebar.nav.dashboard"), icon: "LayoutGrid", group: overview },
    { id: "insights", label: t("sidebar.nav.insights"), icon: "ChartBar", group: observe },
  ].map((item) => ({
    ...item,
    isActive: item.id === activeNav || (item.id === "projects" && activeNav.startsWith("project-")),
  }));
};

/**
 * One row of the sidebar footer's settings menu, standing in for V1's `MenuItem`: a label, a glyph,
 * and either an action or a submenu.
 */
export interface ShellMenuItemDto {
  /** The row's identity - its React key - which the label cannot be: the label is translated. */
  id: string;
  label: string;
  icon: React.ReactNode;
  onSelect?: () => void;
  children?: ShellMenuItemDto[];
}

/**
 * The sidebar's settings menu. This fork drops upstream's update check and Help submenu (docs,
 * Discord, issue tracker all pointed at Ivy's project) and ends with About, which credits it.
 */
export const buildSettingsMenuItems = ({
  onSelectNav,
  t = commonT,
}: {
  onSelectNav: (navId: string) => void;
  t?: TFunction<"common">;
}): ShellMenuItemDto[] => {
  const items: ShellMenuItemDto[] = [
    {
      id: "configuration",
      label: t("sidebar.settingsMenu.configuration"),
      icon: <Construction aria-hidden="true" />,
      onSelect: () => onSelectNav("settings"),
    },
    {
      id: "pull-requests",
      label: t("sidebar.settingsMenu.pullRequests"),
      icon: <GitPullRequest aria-hidden="true" />,
      onSelect: () => onSelectNav("pull-requests"),
    },
    {
      id: "icebox",
      label: t("sidebar.settingsMenu.icebox"),
      icon: <Snowflake aria-hidden="true" />,
      onSelect: () => onSelectNav("icebox"),
    },
  ];

  items.push({
    id: "about",
    label: t("sidebar.settingsMenu.about"),
    icon: <Info aria-hidden="true" />,
    onSelect: () => onSelectNav("about"),
  });

  return items;
};

/** Renders {@link buildSettingsMenuItems} the way V1's `DropDownMenu.Items(...)` renders `MenuItem`s. */
const renderMenuItems = (items: ShellMenuItemDto[]): React.ReactNode =>
  items.map((item) =>
    item.children ? (
      <DropdownMenuSub key={item.id}>
        <DropdownMenuSubTrigger>
          {item.icon}
          {item.label}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>{renderMenuItems(item.children)}</DropdownMenuSubContent>
      </DropdownMenuSub>
    ) : (
      <DropdownMenuItem key={item.id} onSelect={item.onSelect}>
        {item.icon}
        {item.label}
      </DropdownMenuItem>
    ),
  );

interface ShellLayoutProps {
  activeNav: string;
  /**
   * The strip's session panes, which are the `allowDuplicateTabs` apps (review-action runs, and agent
   * terminals once V2 has them) and nothing else. Navigating a page never adds one: V1's strip is the
   * non-closable `$page` tab plus the sessions (`TendrilAppShell.BuildStripTabs`), so the page tab
   * *is* the page.
   */
  sessionTabs?: SessionPane[];
  /**
   * Ignored. It used to be the strip's contents, which is how ordinary pages ended up accumulating
   * there. A page is never a tab, and a session pane only ever comes from
   * {@link ShellLayoutProps.sessionTabs}, so there is nothing a list of nav ids can contribute.
   * Accepted only so callers that still pass it keep compiling; drop it at the call site.
   */
  activeTabs?: string[];
  /** The session pane on top, or null while the page is showing (V1's `selectedIndex`). */
  activeSessionId?: string | null;
  /**
   * One node per entry in {@link ShellLayoutProps.sessionTabs}, in the same order. Every pane stays
   * mounted and only the active one is visible, which is what keeps a review action's terminal
   * running while the reviewer goes back to the plan (V1's `ShowPage` comment).
   */
  sessionContents?: React.ReactNode[];
  /**
   * The page the `$page` tab reveals (V1's `currentApp`), which is `activeNav` unless a session pane
   * is showing over it.
   */
  pageNav?: string;
  serviceInfo: ServiceInfo | null;
  connectionStatus: "online" | "reconnecting" | "offline";
  reconnectCountdown: number;
  onSelectNav: (navId: string) => void;
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  /** V1 `ShowPage`: the `$page` tab was picked, so reveal the page and leave the panes mounted. */
  onShowPage?: () => void;
  onNewPlan: () => void;
  onReconnect: () => void;
  onRestartService?: () => void;
  onRepairService?: () => void;
  onViewDiagnostics?: () => void;
  draftCount?: number;
  reviewCount?: number;
  recommendationsCount?: number;
  jobCount?: number;
  /** Jobs in the Running state, for the top bar's live chip. */
  runningJobCount?: number;
  chatCount?: number;
  /**
   * The contextual list the active app published into the sidebar (V1's `ShellSidebarListSignal`).
   * Absent, or belonging to an app the user has navigated away from, leaves the section holding the
   * full-width Search button V1 shows for every app without a list.
   */
  sidebarList?: ShellSidebarList | null;
  /**
   * A sidebar row click. V1 routes it as `OpenApp(new NavigateArgs(list.AppId,
   * list.BuildSelectArgs(itemId)))`, so the handler gets exactly what V1's shell has: the list's
   * app, the row's id, and the args the list built for it.
   */
  onSelectSidebarItem?: (appId: string, itemId: string, args: unknown) => void;
  /** V1's `showPlanSearchDialog`: what the section's search does when a list supplies no `onSearch`. */
  onPlanSearch?: () => void;
  /**
   * V1's `StartNewChat`, bound to the Chat row unconditionally (`TendrilAppShell.cs:1085`:
   * `.OnNewChat(StartNewChat)`), and only *overridden* by a published list's own `OnNew` when that
   * list is a collapsed rail flyout (`.OnNewChat(chatList.OnNew ?? StartNewChat)`).
   */
  onNewChat?: () => void;
  /**
   * V1's `OpenChat`, which is `ChatLauncher.TargetFor` and therefore consults `chatMode`: the Chat
   * button opens either the chat view or the agent's own terminal. Absent falls back to navigating the
   * chat page, which is the right answer for any host that has no terminal pane to offer.
   */
  onOpenChat?: () => void;
  /** The manager status cards at the foot of the sidebar. */
  projects?: ProjectSummary[];
  jobs?: Job[];
  onOpenProject?: (name: string) => void;
  children: React.ReactNode;
}

/**
 * The app chrome, mirroring V1's `TendrilAppShell.Build()`.
 *
 * The sidebar body is New Plan, then the Chat row, then the nav (V1's
 * `sidebarBody: [newPlanButton, chatButton, nav, section]`). Chat and Inbox are
 * deliberately absent from the nav: V1's `BuildNavItems` drops the agent and chat
 * entries because the dedicated Chat row above the nav reaches them, and drops
 * `footerAppIds` because the Inbox gets its own icon-only footer button
 * (`ShowInboxInFooter`). Settings, Pull Requests, and Icebox are `isVisible: false` apps in
 * V1, reached from the footer's settings menu (`settingsMenuItems`), not the nav.
 */
export const ShellLayout: React.FC<ShellLayoutProps> = ({
  activeNav,
  sessionTabs,
  activeSessionId = null,
  sessionContents,
  pageNav,
  serviceInfo,
  connectionStatus,
  reconnectCountdown,
  onSelectNav,
  onSelectTab,
  onCloseTab,
  onShowPage,
  onNewPlan,
  onReconnect,
  onRestartService,
  onRepairService,
  onViewDiagnostics,
  draftCount,
  reviewCount,
  recommendationsCount,
  jobCount,
  runningJobCount,
  sidebarList = null,
  onSelectSidebarItem,
  onPlanSearch,
  onNewChat,
  projects,
  jobs,
  onOpenProject,
  onOpenChat,
  children,
}) => {
  const { t } = useTranslation("common");

  /* V1 `TendrilAppShell.Build()` line-for-line: a published list is rendered while
     `UsesSidebarList` holds, so moving between two sidebar-section apps (Review to Plans) does not
     blank the sidebar; anything else falls back to the section's own Search button.

     `withNavSelection` is V2's own: the list we retain across a `plan-<id>` nav is one whose
     publisher unmounted on that nav, so its `selectedId` is whatever was selected before the click
     and nothing will ever republish it. Resolving the selection from the nav here -- once, where the
     rail, the section and the flyout all read it -- is what keeps those three from disagreeing. */
  const retained =
    sidebarList && usesSidebarList(sidebarList.appId, activeNav) ? sidebarList : null;
  // No conversation or plan list in the sidebar: chats live inside each project, and the section
  // below is only the search button, which opens the project palette.
  const list: ShellSidebarList | null = null as ShellSidebarList | null;
  void withNavSelection(retained, activeNav);

  /* V1 folds a `CollapsedMenu` list into the Chat row's rail flyout (`chatButton.List(...)`)
     instead of leaving it on the rail as narrow ID chips, and `ShellSidebarSection` drops its own
     rail list for the same flag. Only the rail shows it: `ShellAgentButton` reads `useShell()`. */
  const railFlyoutList = list?.collapsedMenu ? list : null;

  const sectionEvents = ["OnSearch"];
  if (list) {
    sectionEvents.push("OnSelectItem");
    if (list.onNew) sectionEvents.push("OnNew");
    if (list.onRename) sectionEvents.push("OnRenameItem");
    if (list.onDelete) sectionEvents.push("OnDeleteItem");
    if (list.onTogglePin) sectionEvents.push("OnTogglePinItem");
  }

  /* `OnNewChat` is unconditional, as in V1: the Chat row's "New Chat" chord has to work from
     Dashboard, Jobs and every other page, not only from the pages that publish a collapsed list.
     Gating it on `railFlyoutList.onNew` left the chord, and the flyout's own button, inert
     everywhere else. */
  const chatEvents = ["OnOpen", "OnNewChat"];
  if (railFlyoutList) {
    chatEvents.push("OnSelectItem");
    if (railFlyoutList.onRename) chatEvents.push("OnRenameItem");
    if (railFlyoutList.onDelete) chatEvents.push("OnDeleteItem");
    if (railFlyoutList.onTogglePin) chatEvents.push("OnTogglePinItem");
  }

  /** Row actions and the two affordances, shared by the expanded section and the rail flyout. */
  const handleListEvent = (source: ShellSidebarList | null, evt: string, args?: unknown[]) => {
    switch (evt) {
      case "OnSearch":
        // V1: `.OnSearch(list.OnSearch ?? showPlanSearchDialog)` - absent means the plan search.
        (source?.onSearch ?? onPlanSearch)?.();
        return;
      case "OnNew":
        // The section's own affordance, only ever present when the list supplies it.
        source?.onNew?.();
        return;
      case "OnNewChat":
        // V1's `chatList.OnNew ?? StartNewChat`: the flyout list may override, but the shell's
        // handler is what makes the row work on a page that publishes no list at all.
        (source?.onNew ?? onNewChat)?.();
        return;
      case "OnSelectItem": {
        const itemId = firstStringArg(args);
        if (itemId && source) {
          onSelectSidebarItem?.(source.appId, itemId, source.buildSelectArgs(itemId));
        }
        return;
      }
      case "OnRenameItem": {
        // The section sends a rename as one `[id, title]` tuple argument, not two arguments.
        const pair = args?.[0];
        if (Array.isArray(pair) && typeof pair[0] === "string" && typeof pair[1] === "string") {
          source?.onRename?.(pair[0], pair[1]);
        }
        return;
      }
      case "OnDeleteItem": {
        const itemId = firstStringArg(args);
        if (itemId) source?.onDelete?.(itemId);
        return;
      }
      case "OnTogglePinItem": {
        const itemId = firstStringArg(args);
        if (itemId) source?.onTogglePin?.(itemId);
        return;
      }
    }
  };

  const navItems = buildNavItems(
    activeNav,
    {
      plans: draftCount,
      review: reviewCount,
      recommendations: recommendationsCount,
      jobs: jobCount,
    },
    t,
  );

  const settingsMenuItems = buildSettingsMenuItems({ onSelectNav, t });

  /* V1 `BuildStripTabs`: the strip is one non-closable `$page` tab, which reveals the page behind
     the session panes, followed by the session tabs. Nothing else is ever in it - a page is not a
     tab. Session tabs pass no icon, so they get the terminal glyph, and they are closable because
     closing one is how the reviewer says they are done watching. */
  const sessions: SessionPane[] = sessionTabs ?? [];

  /* V1 `TendrilAppShell`: "Terminal panes are reached from the Chats list, so the strip only shows
     the other session tabs (review actions)." A terminal chat is a conversation and belongs beside
     the other conversations in the sidebar; carrying it here too showed one chat twice under two
     names, the sidebar's own title and the strip's generic "Agent".

     Only the *strip* is filtered. `activeSessionIndex` below still indexes the unfiltered
     `sessions`, because that number is what `TendrilShell` uses to pick a pane out of
     `slots.SessionContents`, which the caller builds from the same unfiltered list. */
  const stripSessions = sessions.filter((session) => session.appId !== AGENT_APP_ID);

  const pageNavId = pageNav ?? activeNav;
  const pageAppTitle = appDescriptor(pageNavId)?.title ?? pageNavId;
  /* The page behind the session panes is what the content container pads, not the session on top:
     a pane is its own `.tsh-frame-pane` and never passes through this container. */
  const isPageFullBleed = isFullBleedApp(pageNavId);
  /* V1 `PageTabDisplay` / `PageTabTitle`: the page tab is named after the selected sidebar row, so
     the strip reads "#74 Draft" rather than the generic "Plans", falling back to the app's own
     title. V1 restricts that to the list's own app (`published.AppId == pageAppId`). */
  const pageTitle =
    list && pageNavId === list.appId ? pageTabTitle(pageAppTitle, list) : pageAppTitle;

  // The top bar names the page by its nav row when it has one, so it reads "Work › Plans".
  const pageNavItem = navItems.find((item) => item.id === pageNavId);

  const shellTabs: ShellTabDto[] = [
    { id: PAGE_TAB_ID, title: pageTitle, icon: pageIcon(pageNavId), closable: false },
    ...stripSessions.map((session) => ({ id: session.id, title: session.title })),
  ];

  /* V1's `.HasTabs(stripTabs.Count > 0)` counts session tabs only: the page tab never keeps the
     strip alive on its own, and ShellTabs applies the same rule to its own markup. Keep the two in
     step. A terminal pane alone therefore leaves the strip hidden, as it does in V1. */
  const hasSessionTabs = stripSessions.length > 0;

  /* V1 `SelectedStripTabId`: the active session's id, or the page tab when no session is showing. */
  const activeSession =
    activeSessionId ?? (sessions.some((s) => s.id === activeNav) ? activeNav : null);
  /* V1 `SelectedStripTabId` again: a terminal pane marks *nothing* in the strip, rather than falling
     back to the page tab - the page is not what is on screen, so highlighting it would be a lie. */
  const activeStripSession =
    activeSession && stripSessions.some((session) => session.id === activeSession)
      ? activeSession
      : null;
  const selectedStripTabId = activeStripSession ?? (activeSession ? undefined : PAGE_TAB_ID);
  /* Indexes the UNFILTERED list: `TendrilShell` reads this number off `slots.SessionContents`, which
     holds one node per session pane including the agent terminals the strip leaves out. */
  const activeSessionIndex = activeSession
    ? sessions.findIndex((session) => session.id === activeSession)
    : null;

  const noop = () => {};

  return (
    <div className="flex h-screen h-[var(--app-height,100dvh)] w-screen flex-col overflow-hidden bg-background font-sans text-foreground">
      {/* Top Offline / Reconnection Banner */}
      <OfflineBanner
        status={connectionStatus}
        countdown={reconnectCountdown}
        onReconnect={onReconnect}
      />

      {/* The full service banner only when the daemon is not connected; a healthy one is the top
          bar's service chip instead, so the window stays edge to edge. */}
      {serviceInfo?.state !== "Connected" && (
        <ServiceStatusBanner
          serviceInfo={serviceInfo}
          onRestart={onRestartService}
          onRepair={onRepairService}
          onViewDiagnostics={onViewDiagnostics}
        />
      )}

      {/* The band above the sidebar's brand row, beside the macOS traffic lights, drags the window. */}
      <div className="tauri-drag-strip" data-tauri-drag-region aria-hidden="true" />

      <div className="flex-1 overflow-hidden">
        <TendrilShell
          id="tendril-shell"
          eventHandler={noop}
          hasTabs={hasSessionTabs}
          activeSessionIndex={activeSessionIndex}
          slots={{
            // Phone layout (below 768px): brand and New Plan in a slim top bar, the five main places
            // in a floating tab bar, and this sidebar as a drawer behind the menu button.
            MobileBrand: (
              <>
                <ForgeLogo className="size-[22px]" />
                <span className="truncate">{PRODUCT_NAME}</span>
              </>
            ),
            MobileActions: (
              <button
                type="button"
                onClick={onNewPlan}
                aria-label={t("sidebar.newPlan")}
                className="inline-flex h-9 items-center gap-1.5 rounded-full bg-primary px-3.5 text-[13px] font-semibold text-primary-foreground active:opacity-80"
                data-testid="mobile-new-plan"
              >
                <Plus className="size-4" strokeWidth={2.5} aria-hidden="true" />
                {t("sidebar.newPlanShort")}
              </button>
            ),
            MobileNav: (
              <MobileTabBar
                activeNav={activeNav}
                onSelectNav={onSelectNav}
                onOpenProject={onOpenProject}
                badges={{ plans: draftCount, review: reviewCount }}
              />
            ),
            // The header is the brand row alone. V1 formats the version as "v <x.y.z>".
            //
            // `logo` is V1's `.LogoUrl("/tendril/assets/Tendril.svg")` in `TendrilAppShell`, which
            // V2 had simply never passed - hence the missing mark beside the name. Given as a
            // component rather than a URL so it resolves identically in dev, in the bundle and
            // inside Tauri, where a served path is the thing most likely to differ.
            SidebarHeader: (
              <ShellSidebarHeader
                id="shell-sidebar-header"
                title={PRODUCT_NAME}
                logo={<ForgeLogo />}
                version={
                  serviceInfo?.apiVersion
                    ? t("sidebar.version", { version: serviceInfo.apiVersion })
                    : undefined
                }
                eventHandler={noop}
              />
            ),
            SidebarBody: (
              <>
                {/* `events` is required, not optional decoration: every shell widget gates its
                    callback on `events.includes(...)` so the server can declare which events it
                    subscribed to, and the prop defaults to `[]`. Omitting it here made the primary
                    New Plan button swallow every click. */}
                <ShellNewPlanButton
                  id="new-plan-btn"
                  events={["OnClick"]}
                  eventHandler={onNewPlan}
                />
                <ShellNav
                  id="shell-nav"
                  items={navItems}
                  showDivider
                  events={["OnSelect"]}
                  eventHandler={(_evt: string, _id: string, args?: unknown[]) => {
                    const navId = firstStringArg(args);
                    if (navId) onSelectNav(navId);
                  }}
                />
                {/* V1's `section`, last in `sidebarBody`. With a published list it is that list's
                    header, rows and row actions; without one it is the full-width Search button,
                    which is how V1 keeps plan search reachable from every app's sidebar. */}
                <ShellSidebarSection
                  id="shell-sidebar-section"
                  title={list?.title}
                  items={list?.items ?? []}
                  selectedId={list?.selectedId ?? undefined}
                  searchable={list ? list.searchable !== false : true}
                  searchLabel="Jump to project"
                  newLabel={list?.newLabel}
                  collapsedMenu={list?.collapsedMenu ?? false}
                  events={sectionEvents}
                  eventHandler={(evt: string, _id: string, args?: unknown[]) =>
                    handleListEvent(list, evt, args)
                  }
                />
                {onOpenProject && (
                  <SidebarManagers
                    projects={projects ?? []}
                    jobs={jobs ?? []}
                    activeProject={activeNav.startsWith("project-") ? activeNav.slice("project-".length) : null}
                    onOpenProject={onOpenProject}
                  />
                )}
                <React.Suspense fallback={null}>
                  <SidebarStatus serviceInfo={serviceInfo} connectionStatus={connectionStatus} />
                </React.Suspense>
              </>
            ),
            /* V1's footer is `[settingsMenu, inboxButton]`: the settings cog is a
               DropDownMenu trigger and the Inbox its own button, both icon-only
               (`ShowLabel(!inboxInFooter)` is false whenever the Inbox is in the footer). */
            SidebarFooter: (
              <>
                {/* Inbox before Settings, which is a deliberate divergence from V1: V1's footer is
                    `inboxInFooter ? [settingsMenu, inboxButton] : [settingsMenu]`
                    (`TendrilAppShell.cs:1188`), putting Settings first. The footer is a row while the
                    sidebar is expanded and a column once collapsed (`.tsh-sidebar-footer`, identical
                    CSS in both versions), so this order is what puts Inbox above Settings on the
                    rail. Requested explicitly; not drift. */}
                <ShellSettingsButton
                  id="shell-inbox-btn"
                  label={t("sidebar.inbox")}
                  icon="Inbox"
                  showLabel={false}
                  isActive={activeNav === "inbox"}
                  events={["OnClick"]}
                  eventHandler={() => onSelectNav("inbox")}
                />
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <ShellSettingsButton
                      id="shell-settings-btn"
                      label={t("sidebar.settings")}
                      icon="Settings"
                      showLabel={false}
                      isActive={activeNav === "settings"}
                      eventHandler={noop}
                    />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent side="top" align="start" data-drawer-close className="z-[100]">
                    {renderMenuItems(settingsMenuItems)}
                  </DropdownMenuContent>
                </DropdownMenu>
              </>
            ),
            /* The 16px default, or nothing at all for a full-bleed app. Deciding it here - once,
               from the registry - is what keeps it checkable; a view that wants the frame's edges
               says so in `APP_DESCRIPTORS`, not with classes on its root `<div>`. */
            Content: (
              <>
                <React.Suspense
                  fallback={
                    <div className="h-12 shrink-0 border-b border-border/80" aria-hidden="true" />
                  }
                >
                  <ShellTopBar
                    section={pageNavItem?.group}
                    title={pageNavItem?.label ?? pageTitle}
                    onSearch={onPlanSearch}
                    onAgentSession={onNewChat ?? onOpenChat}
                    onInbox={() => onSelectNav("inbox")}
                    runningJobs={runningJobCount}
                    onJobs={() => onSelectNav("jobs")}
                    serviceInfo={serviceInfo}
                    onRestartService={onRestartService}
                    onRepairService={onRepairService}
                    onViewDiagnostics={onViewDiagnostics}
                  />
                </React.Suspense>
                <main
                  data-testid="shell-content"
                  data-full-bleed={isPageFullBleed}
                  className={isPageFullBleed ? CONTENT_FULL_BLEED_CLASS : CONTENT_PADDED_CLASS}
                >
                  {children}
                </main>
              </>
            ),
            /* V1's `sessionContents`: every session pane stays mounted and only the active one is
               visible, so a review action's terminal keeps its buffer - and keeps running - while
               the reviewer goes back to the plan behind it. */
            SessionContents: sessionContents?.map((pane, index) => (
              <React.Fragment key={sessions[index]?.id ?? index}>{pane}</React.Fragment>
            )),
            /* The strip belongs to the shell frame's bottom edge, not to the content: V1
               hands it to the `tabs` slot and gates the row on `hasTabs`. */
            Tabs: (
              <ShellTabs
                id="shell-tabs"
                tabs={shellTabs}
                selectedId={selectedStripTabId}
                events={["OnSelect", "OnClose"]}
                eventHandler={(evt: string, _id: string, args?: unknown[]) => {
                  const tabId = firstStringArg(args);
                  if (!tabId) return;
                  if (evt === "OnClose") {
                    onCloseTab(tabId);
                    return;
                  }
                  if (evt !== "OnSelect") return;
                  // V1: `if (tabId == PageTabId) ShowPage(); else SelectSession(...)`.
                  if (tabId === PAGE_TAB_ID) onShowPage?.();
                  else onSelectTab(tabId);
                }}
              />
            ),
          }}
        />
      </div>
    </div>
  );
};
