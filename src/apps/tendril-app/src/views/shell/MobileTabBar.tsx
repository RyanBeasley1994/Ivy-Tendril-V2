import React from "react";
import { FolderGit2, GitBranch, LayoutGrid, MessageCircle, type LucideIcon } from "lucide-react";
import { cn } from "@ivy-interactive/components";
import { useTranslation } from "../../i18n";
import { useRecentProjects } from "../../state/recentProjects";

export interface MobileTabBarProps {
  activeNav: string;
  onSelectNav: (navId: string) => void;
  /** Opens a project (its manager chat is the first thing on the page). */
  onOpenProject?: (name: string) => void;
  badges?: { managers?: number };
}

interface Tab {
  id: string;
  icon: LucideIcon;
  label: string;
  badge?: number;
}

/**
 * The phone layout's floating tab bar: where someone on the move goes most (the managers' board,
 * the projects, the repos, and the manager they last talked to), one thumb away. Everything else stays in the drawer. A
 * project's own page counts as Manager, a plan's as Projects, so the bar still says where you are.
 */
export const MobileTabBar: React.FC<MobileTabBarProps> = ({
  activeNav,
  onSelectNav,
  onOpenProject,
  badges = {},
}) => {
  const { t } = useTranslation("common");
  const lastProject = useRecentProjects()[0]?.name;
  const tabs: Tab[] = [
    { id: "dashboard", icon: LayoutGrid, label: t("sidebar.nav.dashboard") },
    {
      id: "projects",
      icon: FolderGit2,
      label: t("sidebar.nav.projects"),
    },
    { id: "git", icon: GitBranch, label: t("sidebar.nav.git") },
    {
      id: "manager",
      icon: MessageCircle,
      label: t("sidebar.nav.manager"),
      badge: badges.managers,
    },
  ];
  const current = activeNav.startsWith("project-") ? "manager" : activeNav.startsWith("plan-") ? "projects" : activeNav;

  return (
    <nav
      aria-label={t("sidebar.mobileTabs")}
      className="mx-auto flex max-w-md items-stretch justify-between gap-1 rounded-[22px] border border-border bg-card/95 p-1.5 shadow-[0_18px_40px_-20px_rgba(0,0,0,0.8)] backdrop-blur"
      data-testid="mobile-tab-bar"
    >
      {tabs.map(({ id, icon: Icon, label, badge }) => {
        const active = current === id;
        return (
          <button
            key={id}
            type="button"
            aria-current={active ? "page" : undefined}
            onClick={() =>
              id === "manager" ? (lastProject && onOpenProject ? onOpenProject(lastProject) : onSelectNav("projects")) : onSelectNav(id)
            }
            className={cn(
              "relative flex min-h-[48px] flex-1 flex-col items-center justify-center gap-0.5 rounded-2xl px-1 text-[10.5px] font-medium transition-colors",
              active
                ? "bg-primary/[0.14] text-success"
                : "text-muted-foreground active:bg-secondary/60",
            )}
          >
            <Icon className="size-[19px]" strokeWidth={active ? 2.2 : 1.8} aria-hidden="true" />
            <span className="max-w-full truncate">{label}</span>
            {badge != null && badge > 0 && (
              <span className="absolute right-[18%] top-1 min-w-[16px] rounded-full bg-primary px-1 text-center font-mono text-[9.5px] leading-4 text-primary-foreground">
                {badge > 99 ? "99+" : badge}
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
};
