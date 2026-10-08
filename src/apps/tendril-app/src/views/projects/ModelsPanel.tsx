import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import type { AgentOption } from "../../types/agents";
import { DEFAULT_OPTION_ID } from "../../types/agents";
import type { EngineChoice, EngineRole, ProjectEngineRoles } from "../../types/projectAssets";
import type { Mission, ProjectSummary } from "../../types/api";
import { agentsApi } from "../../api/agentsApi";
import { bridge } from "../../api/bridge";
import { Card, GhostButton, Label, Pill, PrimaryButton } from "../../components/page/kit";

export const ENGINE_ROLES: { role: EngineRole; label: string; hint: string }[] = [
  { role: "manager", label: "Manager", hint: "The chat you talk to. It decides and delegates." },
  { role: "planner", label: "Planner", hint: "Researches the goal and writes the plan." },
  { role: "worker", label: "Worker", hint: "Does the actual building." },
  { role: "judge", label: "Judge", hint: "Reviews each piece against what was asked." },
  { role: "validator", label: "Validator", hint: "Final check on the whole result." },
];

export const DEFAULT_VALUE = "__default__";

const selectClass =
  "h-8 min-w-0 rounded-lg border border-input bg-muted px-2.5 text-[12.5px] text-foreground outline-none focus:border-primary/50 disabled:opacity-50";

const agentLabel = (a: AgentOption): string =>
  a.localModel ? `${a.label} · local${a.localProvider ? ` (${a.localProvider})` : ""}` : a.label;

/** Each role on its own agent and model, changeable at any time and applied to the project straight away. */
export const ModelsPanel: React.FC<{ project: ProjectSummary; missions: Mission[] }> = ({ project, missions }) => {
  const [agents, setAgents] = React.useState<AgentOption[] | null>(null);
  const [roles, setRoles] = React.useState<ProjectEngineRoles>({});
  const [saving, setSaving] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let live = true;
    agentsApi.listAgents().then((a) => live && setAgents(a)).catch((e) => live && setError(String(e)));
    bridge.getProjectEngine(project.name).then((r) => live && setRoles(r)).catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [project.name]);

  const local = agents?.find((a) => a.localModel);
  const claude = agents?.find((a) => a.id.toLowerCase() === "claude");

  const apply = async (changes: Record<string, EngineChoice | null>, key: string) => {
    setSaving(key);
    setSaved(null);
    setError(null);
    try {
      setRoles(await bridge.setProjectEngine(project.name, changes));
      setSaved(key);
      window.setTimeout(() => setSaved((s) => (s === key ? null : s)), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(null);
    }
  };

  const everyRole = (choice: EngineChoice | null) =>
    Object.fromEntries(ENGINE_ROLES.map((r) => [r.role, choice])) as Record<string, EngineChoice | null>;

  const limited = missions.find((m) => m.project === project.name && m.rateLimit);

  if (!agents) {
    return <div className="p-6 text-[13px] text-muted-foreground">{error ?? "Loading agents…"}</div>;
  }

  return (
    <div className="flex flex-col gap-4 px-4 pb-6 pt-4">
      {limited?.rateLimit && (
        <div className="flex flex-wrap items-center gap-3 rounded-xl border border-warning/40 bg-warning/10 p-4">
          <div className="min-w-0 flex-1 basis-56">
            <div className="text-[13px] font-medium text-foreground">Rate-limited</div>
            <div className="text-[12.5px] text-muted-foreground">
              "{limited.title}" is waiting until{" "}
              {new Date(limited.rateLimit.until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.
            </div>
          </div>
          {local && (
            <PrimaryButton
              size="sm"
              disabled={saving !== null}
              onClick={() =>
                void apply(everyRole({ agent: local.id, model: local.localModel }), "all")
              }
            >
              Move everything to local
            </PrimaryButton>
          )}
        </div>
      )}

      <Card
        title="Engines"
        meta={
          saving ? <Pill tone="info">Saving…</Pill> : saved ? <Pill tone="ok">Saved</Pill> : undefined
        }
        bodyClassName="gap-3 p-4"
      >
        <p className="m-0 text-[12.5px] text-muted-foreground">
          Pick the agent and model for each role. Changes apply to this project straight away: live missions switch
          from their next step, and new missions and tasks use them. Whatever is running right now finishes where
          it is.
        </p>
        <div className="flex flex-wrap gap-2">
          {claude && (
            <GhostButton size="sm" disabled={saving !== null} onClick={() => void apply(everyRole({ agent: claude.id }), "all")}>
              All on {claude.label}
            </GhostButton>
          )}
          {local && (
            <GhostButton
              size="sm"
              disabled={saving !== null}
              onClick={() => void apply(everyRole({ agent: local.id, model: local.localModel }), "all")}
            >
              All on local ({local.localModel})
            </GhostButton>
          )}
          <GhostButton size="sm" disabled={saving !== null} onClick={() => void apply(everyRole(null), "all")}>
            Reset all to default
          </GhostButton>
        </div>
        {!local && (
          <p className="m-0 text-[11.5px] text-muted-foreground">
            No local model is set up on the daemon's machine, so there is no local preset. Point Codex at a local
            model in its config and it will appear here.
          </p>
        )}
        {error && <p className="m-0 text-[12.5px] text-destructive">{error}</p>}
      </Card>

      <div className="flex flex-col gap-2.5">
        {ENGINE_ROLES.map(({ role, label, hint }) => (
          <RoleRow
            key={role}
            label={label}
            hint={hint}
            agents={agents}
            value={roles[role]}
            busy={saving === role}
            onChange={(choice) => void apply({ [role]: choice }, role)}
          />
        ))}
      </div>
    </div>
  );
};

export const RoleRow: React.FC<{
  label: string;
  hint: string;
  agents: AgentOption[];
  value: EngineChoice | undefined;
  busy: boolean;
  onChange: (choice: EngineChoice | null) => void;
}> = ({ label, hint, agents, value, busy, onChange }) => {
  const agent = agents.find((a) => a.id === value?.agent);
  const models = (agent?.models ?? []).filter((m) => m.id !== DEFAULT_OPTION_ID);
  const model = models.find((m) => m.id === value?.model);
  const efforts = (model?.efforts?.length ? model.efforts : agent?.efforts ?? []).filter((e) => e.id !== DEFAULT_OPTION_ID);

  return (
    <div className="flex flex-wrap items-center gap-3 rounded-xl border border-border bg-card px-4 py-3">
      <div className="min-w-[150px] flex-1 basis-40">
        <Label>{label}</Label>
        <div className="mt-0.5 text-[12px] text-muted-foreground">{hint}</div>
      </div>

      <select
        aria-label={`${label} agent`}
        value={value?.agent ?? DEFAULT_VALUE}
        disabled={busy}
        className={cn(selectClass, "w-[190px]")}
        onChange={(e) => {
          if (e.target.value === DEFAULT_VALUE) return onChange(null);
          const next = agents.find((a) => a.id === e.target.value);
          onChange({ agent: e.target.value, ...(next?.localModel ? { model: next.localModel } : {}) });
        }}
      >
        <option value={DEFAULT_VALUE}>Default agent</option>
        {agents.map((a) => (
          <option key={a.id} value={a.id}>{agentLabel(a)}</option>
        ))}
      </select>

      <select
        aria-label={`${label} model`}
        value={value?.model ?? DEFAULT_VALUE}
        disabled={busy || !agent}
        className={cn(selectClass, "w-[210px]")}
        onChange={(e) =>
          value &&
          onChange({
            agent: value.agent,
            ...(e.target.value === DEFAULT_VALUE ? {} : { model: e.target.value }),
          })
        }
      >
        <option value={DEFAULT_VALUE}>Agent default model</option>
        {models.map((m) => (
          <option key={m.id} value={m.id}>{m.displayName}</option>
        ))}
        {value?.model && !model && <option value={value.model}>{value.model}</option>}
      </select>

      {agent?.supportsEffort && efforts.length > 0 && (
        <select
          aria-label={`${label} effort`}
          value={value?.effort ?? DEFAULT_VALUE}
          disabled={busy}
          className={cn(selectClass, "w-[120px]")}
          onChange={(e) =>
            value &&
            onChange({
              agent: value.agent,
              ...(value.model ? { model: value.model } : {}),
              ...(e.target.value === DEFAULT_VALUE ? {} : { effort: e.target.value }),
            })
          }
        >
          <option value={DEFAULT_VALUE}>Default effort</option>
          {efforts.map((ef) => (
            <option key={ef.id} value={ef.id}>{ef.displayName}</option>
          ))}
        </select>
      )}
    </div>
  );
};
