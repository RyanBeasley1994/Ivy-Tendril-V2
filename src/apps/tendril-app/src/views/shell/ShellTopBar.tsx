import React from "react";
import { ArrowLeft, ArrowRight, Bell, ChevronRight, Search, SquareTerminal } from "lucide-react";
import { navigation, useNavigation } from "../../state/navigation";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import type { ServiceInfo } from "../../types/api";
import { ownershipLabel } from "../../components/service/ServiceStatusBanner";

interface ShellTopBarProps {
  /** The page's place in the sidebar, e.g. "Work", or nothing for a page outside the nav. */
  section?: string;
  title: string;
  onSearch?: () => void;
  onAgentSession?: () => void;
  onInbox?: () => void;
  /** Jobs running right now; a chip that opens Jobs when above zero. */
  runningJobs?: number;
  onJobs?: () => void;
  /** The service chip: where the daemon is and the three things you can do to it. */
  serviceInfo?: ServiceInfo | null;
  onRestartService?: () => void;
  onRepairService?: () => void;
  onViewDiagnostics?: () => void;
}

const isMac = () => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

/**
 * The command-center top bar over every page: where you are, and the three things you reach for from
 * anywhere - plan search, a new agent session, and the inbox. Each is a real control that opens the
 * same thing its sidebar counterpart does; nothing here is decoration.
 */
export const ShellTopBar: React.FC<ShellTopBarProps> = ({
  section,
  title,
  onSearch,
  onAgentSession,
  onInbox,
  runningJobs = 0,
  onJobs,
  serviceInfo,
  onRestartService,
  onRepairService,
  onViewDiagnostics,
}) => {
  const { t } = useTranslation("common");
  const { canGoBack, canGoForward } = useNavigation();

  // ⌘[ / ⌘] (Alt+←/→ elsewhere) and the mouse's back/forward buttons step through this session.
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable='true'], .xterm")) return;
      const mac = /Mac|iPhone|iPad/.test(navigator.platform);
      const back = mac ? e.metaKey && e.key === "[" : e.altKey && e.key === "ArrowLeft";
      const fwd = mac ? e.metaKey && e.key === "]" : e.altKey && e.key === "ArrowRight";
      if (back) {
        e.preventDefault();
        navigation.goBack();
      } else if (fwd) {
        e.preventDefault();
        navigation.goForward();
      }
    };
    const onMouse = (e: MouseEvent) => {
      if (e.button === 3) navigation.goBack();
      else if (e.button === 4) navigation.goForward();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, []);

  const navButton =
    "flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground disabled:pointer-events-none disabled:opacity-35";

  return (
    // `data-tauri-drag-region`: with the macOS title bar hidden, the bar's empty space moves the window.
    <header
      data-testid="shell-top-bar"
      data-tauri-drag-region
      className="flex h-12 shrink-0 select-none items-center gap-3 border-b border-border/80 px-5"
    >
      <div className="-ml-1.5 flex shrink-0 items-center gap-0.5">
        <button
          type="button"
          className={navButton}
          disabled={!canGoBack}
          onClick={() => navigation.goBack()}
          aria-label={t("topBar.back")}
          title={`${t("topBar.back")} (${/Mac/.test(navigator.platform) ? "⌘[" : "Alt ←"})`}
          data-testid="nav-back"
        >
          <ArrowLeft className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          className={navButton}
          disabled={!canGoForward}
          onClick={() => navigation.goForward()}
          aria-label={t("topBar.forward")}
          title={`${t("topBar.forward")} (${/Mac/.test(navigator.platform) ? "⌘]" : "Alt →"})`}
          data-testid="nav-forward"
        >
          <ArrowRight className="size-4" aria-hidden="true" />
        </button>
      </div>
      <nav
        aria-label="Breadcrumb"
        data-tauri-drag-region
        className="flex min-w-0 items-center gap-1.5 text-[13px]"
      >
        {section && (
          <>
            <span className="text-muted-foreground">{section}</span>
            <ChevronRight className="size-3.5 text-muted-foreground/60" aria-hidden="true" />
          </>
        )}
        <span className="truncate font-medium text-foreground">{title}</span>
      </nav>

      <div data-tauri-drag-region className="h-full flex-1" />
      <div className="flex items-center gap-2">
        {serviceInfo !== undefined && (
          <ServiceChip
            info={serviceInfo}
            onRestart={onRestartService}
            onRepair={onRepairService}
            onDiagnostics={onViewDiagnostics}
          />
        )}
        {runningJobs > 0 && onJobs && (
          <button
            type="button"
            onClick={onJobs}
            className="flex h-[26px] items-center gap-2 rounded-[7px] border border-border bg-muted px-2.5 text-xs text-foreground transition-colors hover:bg-secondary"
          >
            <span
              className="size-1.5 rounded-full bg-success shadow-[0_0_0_3px_color-mix(in_srgb,var(--success)_22%,transparent)]"
              aria-hidden="true"
            />
            {t("topBar.agentsWorking", { count: runningJobs })}
          </button>
        )}
        {onSearch && (
          <button
            type="button"
            onClick={onSearch}
            className="flex h-8 w-72 items-center gap-2 rounded-[9px] border border-border bg-card px-2.5 text-[12.5px] text-muted-foreground transition-colors hover:border-input hover:text-foreground"
          >
            <Search className="size-3.5" aria-hidden="true" />
            <span className="truncate">{t("topBar.search")}</span>
            <kbd className="ml-auto rounded-[5px] border border-input px-1.5 font-mono text-[10.5px]">
              {isMac() ? "⌘K" : "Ctrl K"}
            </kbd>
          </button>
        )}
        {onAgentSession && (
          <button
            type="button"
            onClick={onAgentSession}
            className="flex h-8 items-center gap-1.5 rounded-lg border border-input bg-secondary px-3 text-[12.5px] text-foreground transition-colors hover:bg-secondary/60"
          >
            <SquareTerminal className="size-3.5 text-primary" aria-hidden="true" />
            {t("topBar.agentSession")}
          </button>
        )}
        {onInbox && (
          <button
            type="button"
            onClick={onInbox}
            aria-label={t("topBar.inbox")}
            title={t("topBar.inbox")}
            className="flex size-8 items-center justify-center rounded-[9px] border border-border bg-card text-muted-foreground transition-colors hover:text-foreground"
          >
            <Bell className="size-[15px]" aria-hidden="true" />
          </button>
        )}
      </div>
    </header>
  );
};

/** The daemon's status as one chip; its menu carries the address and the service actions. */
const ServiceChip: React.FC<{
  info: ServiceInfo | null;
  onRestart?: () => void;
  onRepair?: () => void;
  onDiagnostics?: () => void;
}> = ({ info, onRestart, onRepair, onDiagnostics }) => {
  const { t } = useTranslation("common");
  const connected = info?.state === "Connected";
  const remote = info?.ownership === "Remote";
  const address = `${info?.host || "127.0.0.1"}:${info?.port ?? t("serviceStatus.portUnavailable")}`;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          data-testid="service-chip"
          className="flex h-[26px] items-center gap-2 rounded-[7px] border border-border bg-muted px-2.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
        >
          <span
            aria-hidden="true"
            className={`size-1.5 rounded-full ${
              connected
                ? "bg-success shadow-[0_0_0_3px_color-mix(in_srgb,var(--success)_20%,transparent)]"
                : "bg-destructive"
            }`}
          />
          :{info?.port ?? "—"}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-56">
        <DropdownMenuLabel className="flex flex-col gap-0.5">
          <span className="text-xs font-medium">
            {info?.statusBadge || (connected ? t("serviceStatus.connectedExternal") : info?.state || "—")}
          </span>
          <span className="font-mono text-[11px] font-normal text-muted-foreground">
            {address}
            {info?.ownership ? ` · ${ownershipLabel(t, info.ownership)}` : ""}
          </span>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        {onRestart && !remote && (
          <DropdownMenuItem onSelect={onRestart}>{t("serviceStatus.restart")}</DropdownMenuItem>
        )}
        {onRepair && !remote && (
          <DropdownMenuItem onSelect={onRepair} className="text-warning">
            {t("serviceStatus.repair")}
          </DropdownMenuItem>
        )}
        {onDiagnostics && (
          <DropdownMenuItem onSelect={onDiagnostics}>{t("serviceStatus.diagnostics")}</DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
};
