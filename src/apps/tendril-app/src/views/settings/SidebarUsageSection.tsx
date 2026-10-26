import React from "react";
import { Label, Switch } from "@ivy-interactive/components/ui";
import type { TendrilConfig } from "../../types/api";
import { useTranslation } from "../../i18n";
import { SaveError, SubSection } from "./fields";
import {
  CODING_AGENTS,
  SIDEBAR_USAGE_CHANGED_EVENT,
  SIDEBAR_USAGE_KEY,
  readSidebarUsageAgents,
} from "./codingAgents";
import { normalizeAgentName } from "./projectConfig";

/**
 * Which agents' rate-limit windows the sidebar foot shows. Until something is chosen it is the
 * default agent alone; an agent whose provider publishes no usage is simply drawn as nothing.
 */
export const SidebarUsageSection: React.FC<{
  config: TendrilConfig | null;
  /** Writes one key and re-reads the config (`SettingsView`'s `saveRawKey`). */
  onSaveRaw: (key: string, value: unknown) => Promise<void>;
}> = ({ config, onSaveRaw }) => {
  const { t } = useTranslation("settingsAgents");
  const shown = readSidebarUsageAgents(config);
  const defaultAgent = normalizeAgentName(config?.codingAgent ?? "claude");
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState<string | null>(null);

  const toggle = async (id: string, visible: boolean) => {
    const next = visible ? [...new Set([...shown, id])] : shown.filter((s) => s !== id);
    setSaving(id);
    setError(null);
    try {
      await onSaveRaw(SIDEBAR_USAGE_KEY, next);
      window.dispatchEvent(new Event(SIDEBAR_USAGE_CHANGED_EVENT));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  return (
    <SubSection
      title={t("sidebarUsage.title")}
      hint={t("sidebarUsage.hint")}
      count={shown.length}
      testId="sidebar-usage-visibility"
    >
      <div className="grid gap-2 sm:grid-cols-2">
        {CODING_AGENTS.map((agent) => {
          const id = `sidebar-usage-${agent.id}`;
          return (
            <div
              key={agent.id}
              className="flex items-center gap-3 rounded-lg border border-border px-3 py-2"
            >
              <Switch
                id={id}
                checked={shown.includes(agent.id)}
                disabled={saving !== null}
                onCheckedChange={(checked) => void toggle(agent.id, checked)}
              />
              <Label htmlFor={id} className="flex-1 text-sm text-foreground">
                {agent.label}
              </Label>
              {agent.id === defaultAgent && (
                <span className="text-[11px] text-muted-foreground">{t("harnesses.default")}</span>
              )}
            </div>
          );
        })}
      </div>
      <SaveError message={error} />
    </SubSection>
  );
};
