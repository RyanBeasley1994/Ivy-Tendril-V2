import React from "react";
import { bridge } from "../../api/bridge";
import { agentsApi } from "../../api/agentsApi";
import { useTranslation } from "../../i18n";
import type { ServiceInfo } from "../../types/api";
import type { AgentUsageSnapshot, AgentUsageWindow } from "../../types/agents";
import { CODING_AGENTS } from "../settings/codingAgents";
import { normalizeAgentName } from "../settings/projectConfig";
import { formatCountdown, formatPercent, formatWindow } from "../settings/agentUsage";

/** How often the usage card re-reads the provider: the same cadence as Settings' usage strip. */
const USAGE_REFRESH_MS = 5 * 60 * 1000;

interface SidebarStatusProps {
  serviceInfo: ServiceInfo | null;
  connectionStatus: "online" | "reconnecting" | "offline";
}

/** The window the card reports: the longest one, since that is the limit that ends a week. */
const longestWindow = (snapshot: AgentUsageSnapshot | null): AgentUsageWindow | null =>
  snapshot?.windows.reduce<AgentUsageWindow | null>(
    (longest, w) => (longest == null || w.windowMinutes > longest.windowMinutes ? w : longest),
    null,
  ) ?? null;

/**
 * The foot of the command-center sidebar: the configured agent's rate-limit window, and whether the
 * service is reachable. The usage card is drawn only when the agent's provider publishes usage (Claude
 * and Codex do; Gemini, Copilot and the proxies do not), so it never shows an invented quota.
 */
export const SidebarStatus: React.FC<SidebarStatusProps> = ({ serviceInfo, connectionStatus }) => {
  const { t } = useTranslation("common");
  const [agent, setAgent] = React.useState<string | null>(null);
  const [snapshot, setSnapshot] = React.useState<AgentUsageSnapshot | null>(null);
  const [now, setNow] = React.useState(() => new Date());

  React.useEffect(() => {
    let cancelled = false;
    // Through a resolved promise, so a bridge that throws synchronously (no Tauri, a test) is a
    // rejection like any other and the card simply stays away.
    Promise.resolve()
      .then(() => bridge.getConfig())
      .then((config) => {
        if (!cancelled) setAgent(normalizeAgentName(config?.codingAgent || "claude"));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  React.useEffect(() => {
    if (!agent) return;
    let cancelled = false;
    const read = () => {
      setNow(new Date());
      Promise.resolve()
        .then(() => agentsApi.getUsage(agent))
        .then((next) => !cancelled && setSnapshot(next))
        .catch(() => !cancelled && setSnapshot(null));
    };
    read();
    const timer = window.setInterval(read, USAGE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [agent]);

  const window_ = longestWindow(snapshot);
  const agentLabel = CODING_AGENTS.find((a) => a.id === agent)?.label ?? agent ?? "";
  const used = window_ ? Math.min(100, Math.max(0, window_.usedPercent)) : 0;
  const countdown = window_?.resetsAt ? formatCountdown(window_.resetsAt, now) : "";

  const tone =
    connectionStatus === "online"
      ? "bg-success shadow-[0_0_0_3px_color-mix(in_srgb,var(--success)_20%,transparent)]"
      : connectionStatus === "reconnecting"
        ? "bg-warning"
        : "bg-destructive";

  return (
    <div className="tcc-sidebar-status mt-2 flex shrink-0 flex-col gap-2" data-testid="sidebar-status">
      {window_ && (
        <div className="tcc-usage-card flex flex-col gap-2 rounded-[11px] border border-border bg-card p-3">
          <div className="flex items-center gap-2 text-xs">
            <span className="truncate text-muted-foreground">
              {t("sidebar.usage.title", {
                agent: agentLabel,
                window: formatWindow(window_.windowMinutes),
              })}
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
      )}
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
