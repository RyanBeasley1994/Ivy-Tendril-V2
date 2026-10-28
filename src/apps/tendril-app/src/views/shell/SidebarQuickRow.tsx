import React from "react";
import { ChartBar, LifeBuoy, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { useShell } from "@ivy-interactive/components/tendril";

/**
 * The sidebar's header: Support and Insights side by side with the fold button, in place of the brand
 * row. Support carries the operator's unread ticket count.
 *
 * Folded to the rail it is a column: an always-visible unfold button first, then the two as icons.
 * (The shell's own header made its logo double as the unfold control, revealed only on hover, which
 * read as no button at all.)
 */
export const SidebarQuickRow: React.FC<{
  activeNav: string;
  supportUnread: number;
  supportLabel: string;
  insightsLabel: string;
  onSelectNav: (navId: string) => void;
}> = ({ activeNav, supportUnread, supportLabel, insightsLabel, onSelectNav }) => {
  const { collapsed, toggle } = useShell();
  // Dashboard and Insights are one page, under either address.
  const isActive = (id: string) => activeNav === id || (id === "insights" && activeNav === "dashboard");

  const button = (id: string, label: string, icon: React.ReactNode, badge?: number) => (
    <button
      type="button"
      data-drawer-close
      onClick={() => onSelectNav(id)}
      aria-current={isActive(id) ? "page" : undefined}
      aria-label={label}
      title={collapsed ? label : undefined}
      data-testid={`quick-${id}`}
      className={cn(
        "relative flex h-9 min-w-0 items-center justify-center gap-1.5 rounded-[10px] border text-[12.5px] font-medium transition-colors",
        collapsed ? "w-9 shrink-0" : "flex-1",
        isActive(id)
          ? "border-primary/50 bg-primary/10 text-foreground"
          : "border-border bg-card text-muted-foreground hover:border-primary/30 hover:text-foreground",
      )}
    >
      {icon}
      {!collapsed && <span className="truncate">{label}</span>}
      {badge ? (
        <span
          className={cn(
            "rounded-full bg-primary font-mono font-semibold leading-4 text-primary-foreground",
            collapsed ? "absolute -right-1 -top-1 min-w-4 px-1 text-[9.5px]" : "px-1.5 text-[10px]",
          )}
        >
          {badge}
        </span>
      ) : null}
    </button>
  );

  const fold = (
    <button
      type="button"
      onClick={toggle}
      aria-label={collapsed ? "Open sidebar" : "Close sidebar"}
      title={collapsed ? "Open sidebar (⌘B)" : "Close sidebar (⌘B)"}
      className="inline-flex size-9 shrink-0 items-center justify-center rounded-[10px] text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
    >
      {collapsed ? <PanelLeftOpen className="size-4" aria-hidden="true" /> : <PanelLeftClose className="size-4" aria-hidden="true" />}
    </button>
  );

  return (
    <div
      className={cn("tcc-quick-row flex shrink-0 gap-1.5", collapsed ? "flex-col items-center" : "items-center")}
      data-testid="sidebar-quick-row"
    >
      {collapsed && fold}
      {button("support", supportLabel, <LifeBuoy className="size-3.5 shrink-0" aria-hidden="true" />, supportUnread)}
      {button("insights", insightsLabel, <ChartBar className="size-3.5 shrink-0" aria-hidden="true" />)}
      {!collapsed && fold}
    </div>
  );
};
