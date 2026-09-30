import React from "react";
import { Label, Switch } from "@ivy-interactive/components/ui";
import type { TendrilConfig } from "../../types/api";
import { useTranslation } from "../../i18n";
import { SaveError, SubSection } from "./fields";
import { CODING_AGENTS, HIDDEN_AGENTS_KEY, readHiddenAgents } from "./codingAgents";
import { normalizeAgentName } from "./projectConfig";

/**
 * Which harnesses the agent pickers offer (Create New Plan, and a mission's per-role agents). Hiding
 * one only takes it out of the pickers: the configured default agent always stays, and a job or
 * mission already set to a hidden agent keeps running on it.
 */
export const HarnessVisibilitySection: React.FC<{
  config: TendrilConfig | null;
  /** Writes one key and re-reads the config (`SettingsView`'s `saveRawKey`). */
  onSaveRaw: (key: string, value: unknown) => Promise<void>;
}> = ({ config, onSaveRaw }) => {
  const { t } = useTranslation("settingsAgents");
  const hidden = readHiddenAgents(config);
  const defaultAgent = normalizeAgentName(config?.codingAgent ?? "claude");
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState<string | null>(null);

  const toggle = async (id: string, visible: boolean) => {
    const next = visible ? hidden.filter((h) => h !== id) : [...new Set([...hidden, id])];
    setSaving(id);
    setError(null);
    try {
      await onSaveRaw(HIDDEN_AGENTS_KEY, next);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  return (
    <SubSection
      title={t("harnesses.title")}
      hint={t("harnesses.hint")}
      count={CODING_AGENTS.length - hidden.filter((h) => h !== defaultAgent).length}
      testId="harness-visibility"
    >
      <div className="grid gap-2 sm:grid-cols-2">
        {CODING_AGENTS.map((agent) => {
          const isDefault = agent.id === defaultAgent;
          const visible = isDefault || !hidden.includes(agent.id);
          const id = `harness-visible-${agent.id}`;
          return (
            <div
              key={agent.id}
              className="flex items-center gap-3 rounded-lg border border-border px-3 py-2"
            >
              <Switch
                id={id}
                checked={visible}
                disabled={isDefault || saving !== null}
                onCheckedChange={(checked) => void toggle(agent.id, checked)}
              />
              <Label htmlFor={id} className="flex-1 text-sm text-foreground">
                {agent.label}
              </Label>
              {isDefault && (
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
