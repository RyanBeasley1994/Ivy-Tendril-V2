import React from "react";
import { bridge } from "../../api/bridge";
import { agentsApi } from "../../api/agentsApi";
import { useTranslation } from "../../i18n";
import type { ServiceInfo } from "../../types/api";
import type { AgentUsageSnapshot, AgentUsageWindow } from "../../types/agents";
import {
  CODING_AGENTS,
  SIDEBAR_USAGE_CHANGED_EVENT,
  readSidebarUsageAgents,
} from "../settings/codingAgents";
import { formatCountdown, formatPercent, formatWindow } from "../settings/agentUsage";

/** How often the usage card re-reads the provider: the same cadence as Settings' usage strip. */
const USAGE_REFRESH_MS = 5 * 60 * 1000;

interface SidebarStatusProps {
  serviceInfo: ServiceInfo | null;
  connectionStatus: "online" | "reconnecting" | "offline";
}

/** Every window, shortest first: the five-hour limit reads before the weekly one it sits inside. */
const sortedWindows = (snapshot: AgentUsageSnapshot): AgentUsageWindow[] =>
  [...snapshot.windows].sort((a, b) => a.windowMinutes - b.windowMinutes);

/** One window's line: its length, how much is used, a bar, and when it resets. */
const UsageWindowRow: React.FC<{ window: AgentUsageWindow; now: Date }> = ({ window: w, now }) => {
  const { t } = useTranslation("common");
  const used = Math.min(100, Math.max(0, w.usedPercent));
  const countdown = w.resetsAt ? formatCountdown(w.resetsAt, now) : "";
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2 text-[11.5px]">
        <span className="truncate text-muted-foreground">
          {t("sidebar.usage.window", { window: formatWindow(w.windowMinutes) })}
        </span>
        <span className="ml-auto font-mono text-foreground">{formatPercent(used)}</span>
      </div>
      <div className="h-[5px] overflow-hidden rounded-full bg-secondary">
        <div
          className={`h-full rounded-full ${used >= 90 ? "bg-destructive" : used >= 75 ? "bg-warning" : "bg-primary"}`}
          style={{ width: `${used}%` }}
        />
      </div>
      {countdown && (
        <div className="text-[11px] text-muted-foreground">
          {t("sidebar.usage.resets", { countdown })}
        </div>
      )}
    </div>
  );
};

/**
 * The foot of the command-center sidebar: the rate-limit windows of the agents chosen in Settings (the
 * default agent until something is chosen), and whether the service is reachable. A card is drawn only
 * for an agent whose provider publishes usage (Claude and Codex do; Gemini, Copilot and the proxies do
 * not), so it never shows an invented quota.
 */
export const SidebarStatus: React.FC<SidebarStatusProps> = ({ serviceInfo, connectionStatus }) => {
  const { t } = useTranslation("common");
  const [agents, setAgents] = React.useState<string[]>([]);
  const [snapshots, setSnapshots] = React.useState<Record<string, AgentUsageSnapshot | null>>({});
  const [now, setNow] = React.useState(() => new Date());

  React.useEffect(() => {
    let cancelled = false;
    const readConfig = () => {
      // Through a resolved promise, so a bridge that throws synchronously (no Tauri, a test) is a
      // rejection like any other and the cards simply stay away.
      Promise.resolve()
        .then(() => bridge.getConfig())
        .then((config) => {
          if (!cancelled) setAgents(readSidebarUsageAgents(config));
        })
        .catch(() => {});
    };
    readConfig();
    window.addEventListener(SIDEBAR_USAGE_CHANGED_EVENT, readConfig);
    return () => {
      cancelled = true;
      window.removeEventListener(SIDEBAR_USAGE_CHANGED_EVENT, readConfig);
    };
  }, []);

  // Keyed on the list's contents, so a re-read that returns the same agents does not refetch.
  const agentsKey = agents.join(",");
  React.useEffect(() => {
    if (agents.length === 0) {
      setSnapshots({});
      return;
    }
    let cancelled = false;
    const read = () => {
      setNow(new Date());
      for (const agent of agents) {
        Promise.resolve()
          .then(() => agentsApi.getUsage(agent))
          .then((next) => !cancelled && setSnapshots((prev) => ({ ...prev, [agent]: next })))
          .catch(() => !cancelled && setSnapshots((prev) => ({ ...prev, [agent]: null })));
      }
    };
    read();
    const timer = window.setInterval(read, USAGE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentsKey]);

  const cards = agents.flatMap((agent) => {
    const snapshot = snapshots[agent];
    if (!snapshot || snapshot.windows.length === 0) return [];
    return [{ agent, label: CODING_AGENTS.find((a) => a.id === agent)?.label ?? agent, snapshot }];
  });

  const tone =
    connectionStatus === "online"
      ? "bg-success shadow-[0_0_0_3px_color-mix(in_srgb,var(--success)_20%,transparent)]"
      : connectionStatus === "reconnecting"
        ? "bg-warning"
        : "bg-destructive";

  return (
    <div className="tcc-sidebar-status mt-2 flex shrink-0 flex-col gap-2" data-testid="sidebar-status">
      {cards.map(({ agent, label, snapshot }) => (
        <div
          key={agent}
          className="tcc-usage-card flex flex-col gap-2.5 rounded-[11px] border border-border bg-card p-3"
          data-testid={`sidebar-usage-${agent}`}
        >
          <div className="truncate text-xs font-medium text-foreground">{label}</div>
          {sortedWindows(snapshot).map((w) => (
            <UsageWindowRow key={w.windowMinutes} window={w} now={now} />
          ))}
        </div>
      ))}
      <div className="tcc-service-line flex items-center gap-2 px-2.5 text-[11.5px] text-muted-foreground">
        <span className={`size-1.5 shrink-0 rounded-full ${tone}`} aria-hidden="true" />
        <span className="truncate">{t(`sidebar.status.${connectionStatus}`)}</span>
        {serviceInfo?.port != null && (
          <span className="ml-auto font-mono">:{serviceInfo.port}</span>
        )}
      </div>
    </div>
  );
};
