import * as React from "react";
import { Callout } from "../ui/callout";
import { NativeSelect } from "../ui/native-select";
import { ContentInput } from "../ContentInput";
import { useTranslation } from "@/i18n/uiDialogs";
import { DialogShell } from "./DialogShell";
import { AgentPicker, RoleAgentsPicker } from "./AgentPicker";

/**
 * `CreatePlanDialog.AddProjectActionValue`. Picking it is a navigation, not a project.
 */
export const ADD_PROJECT_VALUE = "__tendril_add_project__";

/** The inset track the dialog's two small segmented choices sit in. */
const SEGMENTED = "inline-flex gap-0.5 rounded-field bg-muted/60 p-0.5";

/** One option of a {@link SEGMENTED} group: raised when chosen, quiet otherwise. */
const segment = (active: boolean) =>
  `rounded-selector px-2.5 py-1 text-xs font-medium transition ${
    active
      ? "bg-background text-foreground shadow-sm"
      : "text-muted-foreground hover:text-foreground"
  }`;

/**
 * The project value that asks CreatePlan to pick the project itself. It is what the job is sent and
 * what {@link defaultProject} compares, so only its label is translated.
 */
export const AUTO_PROJECT = "Auto";

/**
 * `CreatePlanDialog.MaxProjectsForToggleVariant`: up to this many projects the picker is a
 * segmented toggle, above it a plain select.
 */
export const MAX_PROJECTS_FOR_TOGGLE = 6;

export interface ProjectOption {
  value: string;
  label: string;
}

/**
 * `CreatePlanDialog.BuildProjectSelectOptions`: "Auto" leads whenever there is more than one
 * project to choose between (or none configured yet), then the projects, then the escape hatch to
 * settings. With exactly one project there is nothing to decide, so no "Auto".
 */
export function buildProjectOptions(
  projectNames: string[],
  includeAddProject: boolean,
  labels: { auto: string; addProject: string },
): ProjectOption[] {
  const options: ProjectOption[] = [];
  if (projectNames.length > 1 || projectNames.length === 0) {
    options.push({ value: AUTO_PROJECT, label: labels.auto });
  }
  options.push(...projectNames.map((p) => ({ value: p, label: p })));
  if (includeAddProject) {
    options.push({ value: ADD_PROJECT_VALUE, label: labels.addProject });
  }
  return options;
}

/**
 * `CreatePlanDialog._defaultProject`: one project means that project; otherwise the remembered
 * or caller-supplied one if it is still real, and "Auto" when it is not.
 */
export function defaultProject(projectNames: string[], preferred?: string): string {
  if (projectNames.length === 1) return projectNames[0];
  if (preferred === AUTO_PROJECT || (preferred && projectNames.includes(preferred))) {
    return preferred;
  }
  return AUTO_PROJECT;
}

/** A file `ContentInput` read into memory, as its `OnUploadFile` event hands it over. */
export interface CreatePlanUpload {
  name: string;
  base64Data: string;
}

/** A coding agent (harness) the operator can pick. `value` is the agent id the job is started with. */
export interface AgentOption {
  value: string;
  label: string;
}

/** The roles a mission runs, in the order the dialog lists them. */
export const MISSION_ROLES = ["planner", "worker", "judge", "validator"] as const;
export type MissionRole = (typeof MISSION_ROLES)[number];

/** The harness picked for each mission role. An empty string means the configured default. */
export type MissionAgentChoice = Record<MissionRole, string>;

/** What `onSubmit` is given beyond the text and the project. */
export interface CreatePlanSubmitOptions {
  /** The harness for the planning job; `undefined` leaves it to the configured default. */
  agent?: string;
}

export type CreateMode = "plan" | "mission";

/** A machine the plan can be created on, when the app is connected to a remote server. */
export interface MachineOption {
  value: string;
  label: string;
  /** No daemon is running there; the option is shown but cannot be picked. */
  disabled?: boolean;
}

export interface CreatePlanDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /** The configured projects' names, in configured order. */
  projects: string[];
  /** Pre-selected project: the remembered one, or the one a caller (an inbox issue) names. */
  initialProject?: string;
  /** Pre-filled task description. */
  initialDescription?: string;
  /**
   * Dispatches CreatePlan with the trimmed description and the picked project. The app owns the
   * request, the dirty-repo preflight in front of it, and the outcome.
   */
  onSubmit: (
    description: string,
    project: string,
    options?: CreatePlanSubmitOptions,
  ) => void | Promise<void>;
  /**
   * The harnesses to offer. Given, the dialog shows a harness picker (one for a plan, one per role for
   * a mission); omitted, jobs run on the configured default and no picker is shown.
   */
  agentOptions?: AgentOption[];
  /** The configured default agent's label, for the pickers' "Default (…)" entry. */
  defaultAgentLabel?: string;
  /**
   * Starts a mission instead of a plan: an orchestrator breaks the task into milestones, runs and
   * judges each, and validates the whole. Given, the dialog offers a Plan / Mission switch.
   */
  onSubmitMission?: (
    description: string,
    project: string,
    agents: MissionAgentChoice,
  ) => void | Promise<void>;
  /** Which mode the dialog opens in, when missions are offered. */
  initialMode?: CreateMode;
  /**
   * The machines to choose between. Given with two or more entries, the dialog shows a "Create on"
   * picker; the app owns the selection because the projects offered depend on it.
   */
  machines?: MachineOption[];
  selectedMachine?: string;
  onMachineChange?: (machine: string) => void;
  /** Opens project settings, for the picker's "+ Add New Project" entry. Omitted, it is not offered. */
  onAddProject?: () => void;
  /**
   * The host adds the project in a dialog of its own over this one, so choosing "+ Add New Project"
   * leaves this dialog open with what was typed. Unset, it closes first (the Settings hand-off).
   */
  addProjectKeepsOpen?: boolean;
  /** A project to select as soon as it is among `projects`: the one just added over this dialog. */
  selectProject?: string;
  /**
   * The configured coding agent's name, for V1's split-button entry "Chat with <agent>". Offered only
   * together with {@link onContinueInChat}.
   */
  agentLabel?: string;
  /** V1's `OnMenuAction`: discuss the plan with the agent instead of creating it. */
  onContinueInChat?: (description: string, project: string) => void;
  /**
   * Stages a file the operator attached and answers with the path it now lives at, which replaces the
   * bare file name in the description's `[file: …]` reference - V1's `UseUpload` handler, which writes
   * into `Attachments/<uploadSessionId>/` and appends ` [file: <path>]`. Omitted, attachments keep the
   * name `ContentInput` gave them.
   */
  onUploadFile?: (file: CreatePlanUpload) => Promise<string>;
  isBusy?: boolean;
  error?: string | null;
}

/** The first argument of a `ContentInput` event when it is a string (`OnChange`, `OnMenuAction`, …). */
function firstString(args: unknown[] | undefined): string | undefined {
  const first = args?.[0];
  return typeof first === "string" ? first : undefined;
}

/** `OnSubmit`'s `{ value, Value }` payload, or `undefined` when it carries no string. */
function submittedValue(args: unknown[] | undefined): string | undefined {
  const first = args?.[0] as { value?: unknown; Value?: unknown } | undefined;
  const value = first?.value ?? first?.Value;
  return typeof value === "string" ? value : undefined;
}

/** `OnUploadFile`'s `{ name, base64Data }` payload, or `undefined` when it is malformed. */
function uploadArg(args: unknown[] | undefined): CreatePlanUpload | undefined {
  const first = args?.[0] as
    | { name?: unknown; Name?: unknown; base64Data?: unknown; Base64Data?: unknown }
    | undefined;
  const name = first?.name ?? first?.Name;
  const base64Data = first?.base64Data ?? first?.Base64Data;
  return typeof name === "string" && typeof base64Data === "string"
    ? { name, base64Data }
    : undefined;
}

/** Removes one ` [file: …]` reference, spaced or not, as V1's `OnRemoveAttachment` does. */
function withoutFileRef(text: string, path: string): string {
  const ref = ` [file: ${path}]`;
  if (text.includes(ref)) return text.replace(ref, "");
  return text.replace(ref.trim(), "");
}

/**
 * Create New Plan, as V1's `CreatePlanDialog` composes it: a project picker over a single content
 * input that owns its own Create button. There is no priority field - V1 passes `priority: 0` for
 * every plan created here (`onCreatePlan(text, project, 0, uploadSessionId)`), and its
 * `PriorityOptions` never reach the dialog body.
 *
 * `.Width(Size.Rem(30))` on the dialog, and `mobileSheet` for V1's bottom `Sheet` below the mobile
 * breakpoint. No footer: `ContentInput` carries Create, the "Chat with <agent>" split entry and the
 * attachment affordances, as V1's `ContentInput` widget does.
 *
 * Presentational. The dispatch, the dirty-repo preflight and the staging of attachments are the
 * app's; this owns the text, the picker and the parsing of `ContentInput`'s events.
 */
export function CreatePlanDialog({
  isOpen,
  onClose,
  projects,
  initialProject,
  initialDescription = "",
  onSubmit,
  onAddProject,
  addProjectKeepsOpen = false,
  selectProject,
  agentLabel,
  onContinueInChat,
  onUploadFile,
  agentOptions,
  defaultAgentLabel,
  onSubmitMission,
  initialMode = "plan",
  machines,
  selectedMachine,
  onMachineChange,
  isBusy = false,
  error,
}: CreatePlanDialogProps) {
  const { t } = useTranslation("uiDialogs");
  const [mode, setMode] = React.useState<CreateMode>(initialMode);
  const [planAgent, setPlanAgent] = React.useState("");
  const [roleAgents, setRoleAgents] = React.useState<MissionAgentChoice>({
    planner: "",
    worker: "",
    judge: "",
    validator: "",
  });
  const isMission = mode === "mission" && onSubmitMission !== undefined;
  const [description, setDescription] = React.useState(initialDescription);
  const [selectedProject, setSelectedProject] = React.useState(() =>
    defaultProject(projects, initialProject),
  );
  const [localError, setLocalError] = React.useState<string | null>(null);
  const containerRef = React.useRef<HTMLDivElement>(null);

  // V1's `.AutoFocus()` on the ContentInput. `ContentInput` exposes no ref, so the shell is pointed at
  // its textarea through a ref that looks it up when the shell asks.
  const textareaRef = React.useMemo<React.RefObject<HTMLElement | null>>(
    () => ({
      get current() {
        return containerRef.current?.querySelector("textarea") ?? null;
      },
    }),
    [],
  );

  // Seeded on every open, not on prop identity: the caller builds `projects` from its own render
  // state, and re-seeding on it would discard what the operator had typed.
  const projectsRef = React.useRef(projects);
  projectsRef.current = projects;
  React.useEffect(() => {
    if (!isOpen) return;
    setDescription(initialDescription);
    setSelectedProject(defaultProject(projectsRef.current, initialProject));
    setLocalError(null);
    setMode(initialMode);
  }, [isOpen, initialDescription, initialProject, initialMode]);

  // The projects can arrive after the dialog opens; a selection that is no longer offered falls back
  // to the default rather than being sent.
  React.useEffect(() => {
    setSelectedProject((current) =>
      current === AUTO_PROJECT || projects.includes(current)
        ? current
        : defaultProject(projects, initialProject),
    );
  }, [projects, initialProject]);

  // Selected once it is offered, and again only if the host names a different one.
  React.useEffect(() => {
    if (selectProject && projects.includes(selectProject)) setSelectedProject(selectProject);
  }, [selectProject, projects]);

  const options = buildProjectOptions(projects, onAddProject !== undefined, {
    auto: t("createPlan.autoProject"),
    addProject: t("createPlan.addProject"),
  });
  const useToggleVariant = projects.length <= MAX_PROJECTS_FOR_TOGGLE;
  const continueLabel =
    agentLabel && onContinueInChat
      ? t("createPlan.continueInChat", { agent: agentLabel })
      : undefined;

  const handleProjectChange = (value: string) => {
    // `UseEffect` on `selectedProject` in V1: the action value is never a selection, it closes the
    // dialog and takes you to Settings → Projects.
    if (value === ADD_PROJECT_VALUE) {
      if (!addProjectKeepsOpen) onClose();
      onAddProject?.();
      return;
    }
    setSelectedProject(value);
  };

  const handleSubmit = (submitted: string) => {
    if (isBusy) return;
    const text = submitted.trim();
    if (!text) {
      setLocalError(t("createPlan.emptyDescription"));
      return;
    }
    setLocalError(null);
    if (isMission) {
      void onSubmitMission(text, selectedProject, roleAgents);
      return;
    }
    void onSubmit(text, selectedProject, planAgent ? { agent: planAgent } : undefined);
  };

  const defaultLabel = t("createPlan.defaultAgent", { agent: defaultAgentLabel ?? "" });
  // The harness pickers ride in the composer's action row, beside the attach button, as one chip.
  const agentPicker =
    agentOptions && agentOptions.length > 0 ? (
      isMission ? (
        <RoleAgentsPicker
          label={t("createPlan.agents")}
          roles={MISSION_ROLES}
          roleLabel={(role) => t(`createPlan.roles.${role}`)}
          values={roleAgents}
          defaultLabel={defaultLabel}
          defaultShortLabel={t("createPlan.defaultShort")}
          mixedLabel={(count) => t("createPlan.mixedAgents", { count })}
          options={agentOptions}
          onChange={(role, v) => setRoleAgents((current) => ({ ...current, [role]: v }))}
        />
      ) : (
        <AgentPicker
          label={t("createPlan.harness")}
          value={planAgent}
          defaultLabel={defaultLabel}
          options={agentOptions}
          onChange={setPlanAgent}
        />
      )
    ) : undefined;

  const handleUpload = async (file: CreatePlanUpload) => {
    if (!onUploadFile) return;
    try {
      const staged = await onUploadFile(file);
      // `ContentInput` has already put ` [file: <name>]` in the text (its `OnChange` runs before the
      // upload); the staged path is what the plan and the agent can actually open.
      setDescription((current) => current.replace(`[file: ${file.name}]`, `[file: ${staged}]`));
    } catch (err) {
      setDescription((current) => withoutFileRef(current, file.name));
      setLocalError(
        t("attachments.failed", {
          name: file.name,
          error: err instanceof Error ? err.message : String(err),
        }),
      );
    }
  };

  const shownError = error ?? localError;

  return (
    <DialogShell
      isOpen={isOpen}
      onClose={onClose}
      title={isMission ? t("createPlan.missionTitle") : t("createPlan.title")}
      testId="new-plan-modal"
      width="rem30"
      mobileSheet
      initialFocusRef={textareaRef}
    >
      <div ref={containerRef} className="space-y-3" data-testid="new-plan-surface">
        {shownError && (
          <Callout.Error className="mb-2" data-testid="create-plan-error">
            {shownError}
          </Callout.Error>
        )}

        {/* One compact row: what to create on the left, where on the right. Pills rather than three
            stacked full-width bars, so the description - the part being written - leads the dialog. */}
        {(onSubmitMission || (machines && machines.length > 1)) && (
          <div className="flex flex-wrap items-center justify-between gap-2">
            {onSubmitMission && (
              <div
                role="radiogroup"
                aria-label={t("createPlan.modeLabel")}
                className={SEGMENTED}
                data-testid="create-mode"
              >
                {(["plan", "mission"] as const).map((m) => (
                  <button
                    key={m}
                    type="button"
                    role="radio"
                    aria-checked={mode === m}
                    onClick={() => setMode(m)}
                    className={segment(mode === m)}
                  >
                    {m === "plan" ? t("createPlan.modePlan") : t("createPlan.modeMission")}
                  </button>
                ))}
              </div>
            )}

            {machines && machines.length > 1 && (
              <div
                className="ml-auto flex min-w-0 items-center gap-1.5"
                data-testid="machine-picker"
              >
                <span className="shrink-0 text-xs text-muted-foreground">
                  {t("createPlan.machineLabel")}
                </span>
                <div
                  role="radiogroup"
                  aria-label={t("createPlan.machineLabel")}
                  className={`${SEGMENTED} min-w-0`}
                >
                  {machines.map((m) => (
                    <button
                      key={m.value}
                      type="button"
                      role="radio"
                      aria-checked={selectedMachine === m.value}
                      disabled={m.disabled}
                      title={m.disabled ? t("createPlan.machineUnavailable") : m.label}
                      onClick={() => onMachineChange?.(m.value)}
                      className={`${segment(selectedMachine === m.value)} max-w-40 truncate disabled:cursor-not-allowed disabled:opacity-50`}
                    >
                      {m.label}
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
        {isMission && (
          <p className="m-0 text-xs text-muted-foreground" data-testid="mission-hint">
            {t("createPlan.modeMissionHint")}
          </p>
        )}

        {/* `Layout.Vertical().Gap(2) | projectPickerWidget | contentInputWidget` */}
        {useToggleVariant ? (
          <div className="flex items-start gap-2">
            <span className="shrink-0 pt-1.5 text-xs text-muted-foreground">
              {t("createPlan.projectPickerLabel")}
            </span>
            <div
              role="radiogroup"
              aria-label={t("createPlan.projectPickerLabel")}
              className="flex min-w-0 flex-1 flex-wrap gap-1.5"
            >
              {options.map((o) => {
                const isAdd = o.value === ADD_PROJECT_VALUE;
                const isSelected = selectedProject === o.value;
                return (
                  <button
                    key={o.value}
                    type="button"
                    role="radio"
                    aria-checked={isSelected}
                    onClick={() => handleProjectChange(o.value)}
                    className={`rounded-selector px-2.5 py-1 text-xs font-medium transition ${
                      isAdd
                        ? "border border-dashed border-border text-muted-foreground hover:border-foreground/40 hover:text-foreground"
                        : isSelected
                          ? "bg-primary/15 text-primary ring-1 ring-inset ring-primary/40"
                          : "bg-muted/50 text-muted-foreground hover:bg-secondary hover:text-foreground"
                    }`}
                  >
                    {o.label}
                  </button>
                );
              })}
            </div>
          </div>
        ) : (
          <NativeSelect
            id="project-select"
            aria-label={t("createPlan.projectPickerLabel")}
            value={selectedProject}
            onChange={(e) => handleProjectChange(e.target.value)}
          >
            {options.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </NativeSelect>
        )}

        <ContentInput
          id="content-input"
          value={description}
          autoFocus
          submitLabel={isMission ? t("createPlan.submitMission") : t("createPlan.submit")}
          placeholder={t("createPlan.placeholder")}
          menuOptions={continueLabel ? [continueLabel] : []}
          slots={agentPicker ? { LeftActions: agentPicker } : undefined}
          eventHandler={(evt: string, _id: string, args?: unknown[]) => {
            switch (evt) {
              case "OnChange": {
                const text = firstString(args);
                if (text !== undefined) setDescription(text);
                return;
              }
              case "OnSubmit": {
                const text = submittedValue(args);
                if (text === undefined) return;
                setDescription(text);
                handleSubmit(text);
                return;
              }
              case "OnMenuAction": {
                // V1 ignores the entry while the description is blank: there is nothing to discuss.
                if (firstString(args) !== continueLabel || !description.trim()) return;
                onContinueInChat?.(description.trim(), selectedProject);
                return;
              }
              case "OnUploadFile": {
                const file = uploadArg(args);
                if (file) void handleUpload(file);
                return;
              }
              case "OnRemoveAttachment": {
                const path = firstString(args);
                if (path !== undefined) setDescription((current) => withoutFileRef(current, path));
                return;
              }
            }
          }}
        />
      </div>
    </DialogShell>
  );
}
