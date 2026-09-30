import React from "react";
import { Feather, LayoutGrid, MessageCircle, Rocket, ThumbsUp, type LucideIcon } from "lucide-react";
import { cn } from "@ivy-interactive/components";
import { useTranslation } from "../../i18n";

export interface MobileTabBarProps {
  activeNav: string;
  onSelectNav: (navId: string) => void;
  onOpenChat?: () => void;
  badges?: { plans?: number; review?: number; chat?: number };
}

interface Tab {
  id: string;
  icon: LucideIcon;
  label: string;
  badge?: number;
}

/**
 * The phone layout's floating tab bar: the five places a reviewer on the move goes most, one thumb
 * away. Everything else stays in the drawer. A plan's own page counts as Plans, so the bar still
 * says where you are.
 */
export const MobileTabBar: React.FC<MobileTabBarProps> = ({
  activeNav,
  onSelectNav,
  onOpenChat,
  badges = {},
}) => {
  const { t } = useTranslation("common");
  const tabs: Tab[] = [
    { id: "dashboard", icon: LayoutGrid, label: t("sidebar.nav.dashboard") },
    { id: "plans", icon: Feather, label: t("sidebar.nav.plans"), badge: badges.plans },
    { id: "review", icon: ThumbsUp, label: t("sidebar.nav.review"), badge: badges.review },
    { id: "missions", icon: Rocket, label: t("sidebar.nav.missions") },
    { id: "chat", icon: MessageCircle, label: t("appTitles.chat"), badge: badges.chat },
  ];
  const current = activeNav.startsWith("plan-") ? "plans" : activeNav;

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
            onClick={() => (id === "chat" && onOpenChat ? onOpenChat() : onSelectNav(id))}
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
