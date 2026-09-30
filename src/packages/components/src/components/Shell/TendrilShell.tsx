import React, { useCallback, useEffect, useRef, useState } from "react";
import { Menu } from "lucide-react";
import { ShellContext } from "./ShellContext.tsx";
import { type ShellWidgetProps } from "./types.ts";
import {
  useResizableSidebar,
  readStoredWidth as readStoredWidthHelper,
  writeStoredWidth as writeStoredWidthHelper,
} from "../../hooks/use-resizable-sidebar";
import { useShortcut } from "../../lib/useShortcut";
import { TooltipScope } from "../ui/TuiTooltip";
import { useTranslation } from "@/i18n/uiShell";
import "./shell.css";

interface TendrilShellProps extends ShellWidgetProps {
  collapsed?: boolean;
  activeSessionIndex?: number | null;
  hasTabs?: boolean;
  slots?: {
    SidebarHeader?: React.ReactNode;
    SidebarBody?: React.ReactNode;
    SidebarFooter?: React.ReactNode;
    Content?: React.ReactNode;
    SessionContents?: React.ReactNode;
    Tabs?: React.ReactNode;
    Hidden?: React.ReactNode;
    /** Phone layout: the brand in the top bar, beside the menu button. */
    MobileBrand?: React.ReactNode;
    /** Phone layout: the top bar's right-hand actions (New plan, inbox). */
    MobileActions?: React.ReactNode;
    /** Phone layout: the bottom tab bar. */
    MobileNav?: React.ReactNode;
  };
}

/** Below this width the sidebar becomes a drawer and the frame goes edge to edge. Mirrors `shell.css`. */
export const MOBILE_SHELL_QUERY = "(max-width: 767px)";

/** Whether the shell is in its phone layout. */
export function useIsMobileShell(): boolean {
  const [mobile, setMobile] = useState(
    () => typeof window !== "undefined" && !!window.matchMedia?.(MOBILE_SHELL_QUERY).matches,
  );
  useEffect(() => {
    const media = window.matchMedia?.(MOBILE_SHELL_QUERY);
    if (!media) return;
    const update = () => setMobile(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return mobile;
}

/** What, clicked in the drawer, is a navigation: picking it closes the drawer. */
const DRAWER_CLOSERS =
  ".tsh-nav-item, .tsh-section-item, .tsh-list-row, [data-drawer-close], a[href]";

export const SIDEBAR_COLLAPSED_STORAGE_KEY = "tendril.shell.sidebarCollapsed";
export const SIDEBAR_WIDTH_STORAGE_KEY = "tendril.shell.sidebarWidth";
export const DEFAULT_SIDEBAR_WIDTH = 256;
export const MIN_SIDEBAR_WIDTH = 200;
export const MAX_SIDEBAR_WIDTH = 640;

const readStoredCollapsed = (): boolean | null => {
  try {
    const raw = window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY);
    if (raw === "true") return true;
    if (raw === "false") return false;
    return null;
  } catch {
    return null;
  }
};

const writeStoredCollapsed = (collapsed: boolean) => {
  try {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, String(collapsed));
  } catch {
    /* storage unavailable (private mode, sandboxed host): the state just doesn't persist */
  }
};

export function readStoredWidth(): number | null {
  return readStoredWidthHelper(SIDEBAR_WIDTH_STORAGE_KEY, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
}

export function writeStoredWidth(width: number | null): void {
  writeStoredWidthHelper(SIDEBAR_WIDTH_STORAGE_KEY, width);
}

/**
 * The Tendril app chrome: sidebar (expanded / icon rail) and one rounded,
 * bordered container holding the white content surface with the session tab
 * strip inside its bottom edge. Collapse is client-side for a smooth
 * animation; the host is notified through OnCollapsedChanged so the state
 * can be persisted, and remembered locally so a reload does not flash the
 * other width. Session panes all stay mounted - only the active one is
 * visible - so agent terminals keep their buffers when switching tabs. The
 * Hidden slot hosts zero-size utility widgets (shortcut ghosts, chunk
 * warm-ups) without letting them paint.
 */
export const TendrilShell: React.FC<TendrilShellProps> = ({
  id,
  events = [],
  eventHandler,
  collapsed: collapsedProp = false,
  activeSessionIndex,
  hasTabs = false,
  slots,
}) => {
  const { t } = useTranslation("uiShell");
  const [collapsed, setCollapsed] = useState(() => {
    const stored = readStoredCollapsed();
    return stored != null ? stored : collapsedProp;
  });
  const {
    width: sidebarWidth,
    isDragging,
    separatorProps,
  } = useResizableSidebar({
    storageKey: SIDEBAR_WIDTH_STORAGE_KEY,
    defaultWidth: DEFAULT_SIDEBAR_WIDTH,
    minWidth: MIN_SIDEBAR_WIDTH,
    maxWidth: MAX_SIDEBAR_WIDTH,
  });
  const mobile = useIsMobileShell();
  const [drawerOpen, setDrawerOpen] = useState(false);
  // Leaving the phone layout (rotating a tablet, widening a window) must not leave a drawer open
  // over the desktop sidebar.
  useEffect(() => {
    if (!mobile) setDrawerOpen(false);
  }, [mobile]);
  useEffect(() => {
    if (!drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDrawerOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [drawerOpen]);
  // The drawer always shows full labels: the desktop rail is not a phone layout.
  const effectiveCollapsed = mobile ? false : collapsed;
  const prevPropRef = useRef(collapsedProp);
  if (collapsedProp !== prevPropRef.current) {
    prevPropRef.current = collapsedProp;
    if (collapsedProp !== collapsed) setCollapsed(collapsedProp);
  }

  const toggle = useCallback(() => {
    setCollapsed((prev) => {
      const next = !prev;
      writeStoredCollapsed(next);
      if (events.includes("OnCollapsedChanged")) {
        eventHandler("OnCollapsedChanged", id, [next]);
      }
      return next;
    });
  }, [events, eventHandler, id]);

  useShortcut("tendril-shell:toggle-sidebar", "mod+b", toggle, {
    // Re-registers when the language changes, so the shortcuts help panel follows it.
    description: t("shell.toggleSidebarShortcut"),
    skipInInputs: true,
    // A toggle is not idempotent: two genuine presses inside the registry's 300ms debounce window
    // must still flip the state twice (open -> closed -> open), not collapse into one fire.
    debounce: false,
  });

  const sessionPanes = React.Children.toArray(slots?.SessionContents ?? []);
  const hasActiveSession =
    activeSessionIndex != null &&
    activeSessionIndex >= 0 &&
    activeSessionIndex < sessionPanes.length;

  return (
    <TooltipScope>
      <div
        /* No `remove-parent-padding` class here. It is Ivy's opt-out for full-bleed widgets, but it only
         works through the `:has(> .remove-parent-padding)` rules in the framework's `index.css`, and V2
         ships no stylesheet implementing them — so carrying it made these components *look* as though
         they handled their own inset while the shell went on padding them anyway. V2 decides full-bleed
         in one place instead: `AppDescriptor.fullBleed` in the app registry, read by `ShellLayout`. */
        className="tsh-root"
        data-collapsed={effectiveCollapsed}
        data-mobile={mobile}
        data-drawer-open={drawerOpen}
        data-resizing={isDragging}
        style={
          {
            "--tsh-sidebar-width": `${sidebarWidth}px`,
          } as React.CSSProperties
        }
      >
        {mobile && (
          <div className="tsh-mobile-bar">
            <button
              type="button"
              className="tsh-mobile-menu"
              aria-label={t("shell.mobileMenu")}
              aria-expanded={drawerOpen}
              onClick={() => setDrawerOpen((open) => !open)}
            >
              <Menu size={20} aria-hidden="true" />
            </button>
            <div className="tsh-mobile-brand">{slots?.MobileBrand}</div>
            <div className="tsh-mobile-actions">{slots?.MobileActions}</div>
          </div>
        )}
        {mobile && drawerOpen && (
          <div className="tsh-backdrop" aria-hidden="true" onClick={() => setDrawerOpen(false)} />
        )}
        <ShellContext.Provider value={{ collapsed: effectiveCollapsed, toggle }}>
          <div
            className="tsh-sidebar"
            aria-hidden={mobile && !drawerOpen ? true : undefined}
            onClickCapture={(e) => {
              if (!mobile) return;
              const target = e.target as HTMLElement | null;
              if (target?.closest(DRAWER_CLOSERS)) {
                // After the click has done its work.
                setTimeout(() => setDrawerOpen(false), 0);
              }
            }}
          >
            <div className="tsh-sidebar-header">{slots?.SidebarHeader}</div>
            <div className="tsh-sidebar-body">{slots?.SidebarBody}</div>
            <div className="tsh-sidebar-footer">{slots?.SidebarFooter}</div>
            {!effectiveCollapsed && !mobile && (
              <div
                className="tsh-sidebar-resizer"
                {...separatorProps}
                aria-label={t("shell.resizer.ariaLabel")}
                title={t("shell.resizer.title")}
              />
            )}
          </div>
        </ShellContext.Provider>
        {/* The rail is a property of the sidebar, not of the app inside the frame: content
          widgets that read `useShell()` must never render their own collapsed variant. */}
        <ShellContext.Provider value={{ collapsed: false, toggle }}>
          <div className="tsh-main">
            <div className="tsh-container">
              <div className="tsh-frame" data-has-tabs={hasTabs}>
                <div className="tsh-frame-pane" data-active={!hasActiveSession}>
                  {slots?.Content}
                </div>
                {sessionPanes.map((pane, index) => (
                  <div
                    className="tsh-frame-pane"
                    data-active={hasActiveSession && index === activeSessionIndex}
                    key={(React.isValidElement(pane) && pane.key) || index}
                  >
                    {pane}
                  </div>
                ))}
              </div>
              {hasTabs && slots?.Tabs && <div className="tsh-tabs-row">{slots.Tabs}</div>}
            </div>
            {mobile && slots?.MobileNav && <div className="tsh-mobile-nav">{slots.MobileNav}</div>}
          </div>
        </ShellContext.Provider>
        {slots?.Hidden && <div style={{ display: "none" }}>{slots.Hidden}</div>}
      </div>
    </TooltipScope>
  );
};
