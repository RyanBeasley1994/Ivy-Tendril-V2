import React from "react";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";
import type { AgentOption } from "../../types/agents";
import type { EngineChoice, GlobalEngine } from "../../types/projectAssets";
import { agentsApi } from "../../api/agentsApi";
import { bridge } from "../../api/bridge";
import { Button } from "@ivy-interactive/components/ui";
import { ENGINE_ROLES, RoleRow } from "../projects/ModelsPanel";
import { SettingsSection } from "./fields";

/**
 * The agent for each role across every project, and the chain of agents that take over when one hits a
 * rate limit. A project's own Models tab overrides a role; a role set nowhere runs on the default agent.
 * Changes apply at once to live missions and managers of projects that do not override them.
 */
export const GlobalEnginesSection: React.FC = () => {
  const [agents, setAgents] = React.useState<AgentOption[] | null>(null);
  const [engine, setEngine] = React.useState<GlobalEngine>({ roles: {}, fallbacks: [] });
  const [busy, setBusy] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let live = true;
    agentsApi.listAgents().then((a) => live && setAgents(a)).catch((e) => live && setError(String(e)));
    bridge.getGlobalEngine().then((g) => live && setEngine(g)).catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, []);

  const save = async (change: { roles?: Record<string, EngineChoice | null>; fallbacks?: EngineChoice[] }, key: string) => {
    setBusy(key);
    setError(null);
    setSaved(false);
    try {
      setEngine(await bridge.setGlobalEngine(change));
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2200);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const move = (from: number, to: number) => {
    const next = [...engine.fallbacks];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    void save({ fallbacks: next }, `fb-${to}`);
  };

  return (
    <SettingsSection
      title="Agents by role"
      hint="The agent and model each role runs on, for every project that doesn't set its own (a project's Models tab overrides these). Changes apply straight away to live missions and managers."
      testId="global-engines-card"
    >
      {!agents ? (
        <p className="text-sm text-muted-foreground">{error ?? "Loading agents…"}</p>
      ) : (
        <div className="space-y-6">
          {(engine.limited ?? []).length > 0 && (
            <div className="rounded-xl border border-warning/40 bg-warning/10 p-3 text-[12.5px]" data-testid="limited-agents">
              {(engine.limited ?? []).map((l) => (
                <div key={l.agent}>
                  <span className="font-medium text-foreground">{l.agent}</span> is rate limited until{" "}
                  {new Date(l.until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.{" "}
                  {engine.fallbacks.length > 0
                    ? "New work is on the fallback until then, and moves back by itself."
                    : "With no fallback, work waits for it."}
                </div>
              ))}
            </div>
          )}
          <div className="flex flex-col gap-2.5">
            {ENGINE_ROLES.map(({ role, label, hint }) => (
              <RoleRow
                key={role}
                label={label}
                hint={hint}
                agents={agents}
                value={engine.roles[role]}
                busy={busy === role}
                onChange={(choice) => void save({ roles: { [role]: choice } }, role)}
              />
            ))}
          </div>

          <div className="space-y-2.5">
            <div>
              <div className="text-sm font-medium">If an agent hits a rate limit</div>
              <p className="mt-0.5 text-xs text-muted-foreground">
                Work that gets rate limited moves to the next agent in this list and carries on, instead of waiting for
                the limit to reset. The first row is tried first. With no fallbacks, work waits.
              </p>
            </div>
            {engine.fallbacks.map((fallback, index) => (
              <div key={`${fallback.agent}-${fallback.model ?? ""}-${index}`} className="flex items-center gap-2">
                <span className="w-5 text-center font-mono text-[11px] text-muted-foreground">{index + 1}</span>
                <div className="min-w-0 flex-1">
                  <RoleRow
                    label={index === 0 ? "First fallback" : `Fallback ${index + 1}`}
                    hint=""
                    agents={agents}
                    value={fallback}
                    busy={busy === `fb-${index}`}
                    onChange={(choice) => {
                      const next = [...engine.fallbacks];
                      if (choice) next[index] = choice;
                      else next.splice(index, 1);
                      void save({ fallbacks: next }, `fb-${index}`);
                    }}
                  />
                </div>
                <div className="flex flex-col">
                  <Button type="button" variant="ghost" size="sm" disabled={index === 0 || busy !== null} onClick={() => move(index, index - 1)} aria-label="Move up">
                    <ArrowUp className="size-3.5" aria-hidden />
                  </Button>
                  <Button type="button" variant="ghost" size="sm" disabled={index === engine.fallbacks.length - 1 || busy !== null} onClick={() => move(index, index + 1)} aria-label="Move down">
                    <ArrowDown className="size-3.5" aria-hidden />
                  </Button>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={busy !== null}
                  onClick={() => void save({ fallbacks: engine.fallbacks.filter((_, i) => i !== index) }, `fb-${index}`)}
                  aria-label="Remove fallback"
                >
                  <Trash2 className="size-4" aria-hidden />
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy !== null || agents.length === 0}
              onClick={() => {
                const taken = new Set(engine.fallbacks.map((f) => f.agent));
                const next = agents.find((a) => !taken.has(a.id)) ?? agents[0];
                void save(
                  { fallbacks: [...engine.fallbacks, { agent: next.id, ...(next.localModel ? { model: next.localModel } : {}) }] },
                  "fb-add",
                );
              }}
              data-testid="add-fallback"
            >
              <Plus className="size-4" aria-hidden /> Add a fallback
            </Button>
          </div>

          {error && <p className="text-xs text-destructive">{error}</p>}
          {saved && <p className="text-xs text-muted-foreground">Saved.</p>}
        </div>
      )}
    </SettingsSection>
  );
};
