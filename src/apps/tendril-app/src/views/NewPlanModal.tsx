import React, { useEffect, useRef, useState } from "react";
import {
  CreatePlanDialog,
  DirtyRepoDialog,
  AUTO_PROJECT,
  type CreatePlanSubmitOptions,
  type CreatePlanUpload,
  type MissionAgentChoice,
  type SyncRepoPolicy,
} from "@ivy-interactive/components/dialogs";
import type {
  Machine,
  MachineTarget,
  Mission,
  MissionAgents,
  ProjectSummary,
  RepoStatus,
  StartJobArgs,
  StartJobResponse,
  TendrilConfig,
} from "../types/api";
import { useTranslation } from "../i18n";
import { describeBridgeError } from "../types/api";
import { bridge } from "../api/bridge";
import { jobsStore } from "../state/jobsStore";
import { notificationsStore } from "../state/notificationsStore";
import { uiStore } from "../state/uiStore";
import { NEW_CHAT_TITLE } from "../state/chatLauncher";
import { CODING_AGENTS, visibleAgents } from "./settings/codingAgents";
import { newUploadSessionId } from "./dialogs/useDialogAttachments";

interface NewPlanModalProps {
  isOpen: boolean;
  onClose: () => void;
  projects: ProjectSummary[];
  onJobStarted?: (res: StartJobResponse) => void;
  /**
   * A mission was created from the dialog's Mission mode. Given, the dialog offers the Plan / Mission
   * switch; omitted, it creates plans only.
   */
  onMissionCreated?: (mission: Mission) => void;
  /** Opens project settings, for the picker's "+ Add New Project" entry. Omitted, the entry is not offered. */
  onAddProject?: () => void;
  /** `onAddProject` opens a dialog over this one, so this stays open (see `CreatePlanDialog`). */
  addProjectKeepsOpen?: boolean;
  /** A project to select once it is listed: the one just added. */
  selectProject?: string;
  initialTitle?: string;
  initialDescription?: string;
  initialProject?: string;
  initialSourceUrl?: string;
  /** Which mode the dialog opens in, when missions are offered. */
  initialMode?: "plan" | "mission";
}

/**
 * V1 `CreatePlanDialog.BuildAgentPrompt`: the seed for "Chat with <agent>". Sent to the agent as the
 * conversation's first message without the operator editing it, so it stays English - it is text
 * Tendril sends on the operator's behalf, not copy on screen.
 */
export function buildCreatePlanAgentPrompt(project: string, description: string): string {
  const trimmed = description.trim();
  if (!project || project === AUTO_PROJECT) {
    return `I want to discuss creating a Tendril plan from this description: "${trimmed}". Determine the most appropriate project for it yourself.`;
  }
  return `I want to discuss creating a Tendril plan for the project ${project} from this description: "${trimmed}"`;
}

/** V1 `AgentBranding.For(settings.CodingAgent).Label`, for "Chat with <agent>". */
function agentLabelFor(agentId: string | undefined): string {
  const id = (agentId ?? "").trim() || "claude";
  return CODING_AGENTS.find((agent) => agent.id === id)?.label ?? id;
}

/**
 * A mission's title, from the task: its first line, cut at a word boundary. The goal keeps the whole
 * text, so nothing is lost; the title only has to name the mission in a list.
 */
export function missionTitleFrom(description: string, max = 72): string {
  const firstLine = description.trim().split(/\r?\n/)[0]?.trim() ?? "";
  if (firstLine.length <= max) return firstLine;
  const cut = firstLine.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${(space > max / 2 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/** The dialog's per-role picks as the daemon takes them: an empty pick means "the default agent". */
export function toMissionAgents(choice: MissionAgentChoice): MissionAgents {
  const agents: MissionAgents = {};
  for (const role of ["planner", "worker", "judge", "validator"] as const) {
    if (choice[role]) agents[role] = { agent: choice[role] };
  }
  return agents;
}

const combine = (title: string, description: string): string =>
  title ? (description ? `${title}\n\n${description}` : title) : description;

/**
 * The connected half of `CreatePlanDialog`, and V1's `CreatePlanDialogLauncher` around it.
 *
 * The dialog owns the picker, the text and `ContentInput`'s events. What lives here is everything
 * that reaches the daemon:
 *
 * - **The dispatch.** CreatePlan with `priority: 0` (V1's dialog has no priority field), the
 *   caller's `sourceUrl`, and the upload session when anything was attached.
 * - **The dirty-repo preflight** (V1 `UsePreflightCheck`): for a named project, each repo's
 *   uncommitted work is read before the job starts, and a dirty one swaps the dialog for
 *   `DirtyRepoDialog` - "Create Without Syncing", or *Sync Repos*, which chains one SyncRepo job per
 *   dirty repo and has CreatePlan wait for them (`waitForJobs`), as V1's `LaunchWithSync` does.
 *   "Auto" has no repos to check until the agent has picked a project, which is V1's behaviour too
 *   (`GetProject("Auto")` is null). A preflight that fails to answer never blocks the plan.
 * - **Attachments.** `ContentInput` hands over bytes; they are staged under this opening's upload
 *   session (`Attachments/<id>/`), which CreatePlan's `uploadSessionId` promotes into the plan folder.
 * - **Continue in chat.** V1's split-button entry: a new chat seeded with `BuildAgentPrompt`.
 */
export const NewPlanModal: React.FC<NewPlanModalProps> = ({
  isOpen,
  onClose,
  projects,
  onJobStarted,
  onMissionCreated,
  onAddProject,
  addProjectKeepsOpen,
  selectProject,
  initialTitle = "",
  initialDescription = "",
  initialProject = "",
  initialSourceUrl = "",
  initialMode = "plan",
}) => {
  const { t } = useTranslation("missions");
  const [isBusy, setIsBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dirtyRepos, setDirtyRepos] = useState<RepoStatus[] | null>(null);
  const [pending, setPending] = useState<StartJobArgs | null>(null);
  const [agentLabel, setAgentLabel] = useState<string | undefined>(undefined);
  /** The config as last read, for the harnesses Settings hides from the pickers. */
  const [config, setConfig] = useState<TendrilConfig | null>(null);
  /**
   * What was submitted, so the dialog comes back with it when the dirty-repo step hands control back
   * after a failed dispatch: the dialog is unmounted while `DirtyRepoDialog` shows, and would
   * otherwise re-seed from the caller's prefill.
   */
  const [draft, setDraft] = useState<{ description: string; project: string } | null>(null);
  /** The machines to choose between; empty (no picker) unless the app is connected to a remote. */
  const [machines, setMachines] = useState<Machine[]>([]);
  const [machine, setMachine] = useState<MachineTarget>("remote");
  /** This machine's projects, fetched when it is picked: the caller's list is the remote's. */
  const [localProjects, setLocalProjects] = useState<ProjectSummary[] | null>(null);
  const uploadSessionId = useRef(newUploadSessionId());
  const uploaded = useRef(false);

  useEffect(() => {
    if (!isOpen) return;
    uploadSessionId.current = newUploadSessionId();
    uploaded.current = false;
    setIsBusy(false);
    setError(null);
    setDirtyRepos(null);
    setPending(null);
    setDraft(null);
    setMachine("remote");
    setLocalProjects(null);
    let cancelled = false;
    void Promise.resolve()
      .then(() => bridge.listMachines())
      .then((list) => {
        if (!cancelled) setMachines(Array.isArray(list) ? list : []);
      })
      .catch(() => {
        if (!cancelled) setMachines([]);
      });
    void Promise.resolve()
      .then(() => bridge.getConfig())
      .then((config) => {
        if (cancelled) return;
        setAgentLabel(agentLabelFor(config?.codingAgent));
        setConfig(config ?? null);
      })
      .catch(() => {
        if (!cancelled) setAgentLabel(agentLabelFor(undefined));
      });
    return () => {
      cancelled = true;
    };
  }, [isOpen]);

  const remoteHost = machines.find((m) => m.id === "remote")?.host ?? "";
  /** Only sent when the operator picked this machine while connected to a remote. */
  const target: MachineTarget | undefined =
    machines.length > 1 && machine === "local" ? "local" : undefined;
  const elsewhere = target === "local";
  const machineLabel = machine === "local" ? t("machines.local") : remoteHost;
  const shownProjects = elsewhere ? (localProjects ?? []) : projects;

  const handleMachineChange = (next: string) => {
    const picked: MachineTarget = next === "local" ? "local" : "remote";
    setMachine(picked);
    if (picked === "local" && localProjects === null) {
      void bridge
        .listProjects("local")
        .then((list) => setLocalProjects(Array.isArray(list) ? list : []))
        .catch((err) => {
          setLocalProjects([]);
          setError(describeBridgeError(err));
        });
    }
  };

  const finish = () => {
    setIsBusy(false);
    setDirtyRepos(null);
    setPending(null);
    onClose();
  };

  /** V1 `LaunchCreatePlan` / the tail of `LaunchWithSync`. */
  const launch = async (args: StartJobArgs, waitForJobs: string[] = []) => {
    setIsBusy(true);
    setError(null);
    try {
      const res = await jobsStore.startJob(
        {
          ...args,
          ...(waitForJobs.length > 0 ? { waitForJobs } : {}),
        },
        target,
      );
      if (elsewhere) {
        // The job lives on the other daemon; opening it here would look it up on the wrong one.
        notificationsStore.notifySuccess(
          res.jobId,
          t("machines.startedElsewhere", { machine: machineLabel }),
        );
      } else {
        onJobStarted?.(res);
      }
      finish();
    } catch (err) {
      // Back to the dialog, carrying the failure, with the operator's text still in it.
      setIsBusy(false);
      setDirtyRepos(null);
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleSubmit = async (
    description: string,
    project: string,
    options?: CreatePlanSubmitOptions,
  ) => {
    if (isBusy) return;
    setDraft({ description, project });
    const args: StartJobArgs = {
      type: "CreatePlan",
      project,
      description,
      priority: 0,
      ...(options?.agent ? { agent: options.agent } : {}),
      sourceUrl: initialSourceUrl.trim() || undefined,
      ...(uploaded.current ? { uploadSessionId: uploadSessionId.current } : {}),
    };

    if (project && project !== AUTO_PROJECT) {
      setIsBusy(true);
      setError(null);
      let dirty: RepoStatus[] = [];
      try {
        const status = await bridge.getProjectRepoStatus(project, target);
        dirty = Array.isArray(status) ? status.filter((repo) => repo.isDirty) : [];
      } catch {
        // Unknown is not dirty: the guard never blocks plan creation on an unreadable repo.
      }
      if (dirty.length > 0) {
        setIsBusy(false);
        setPending(args);
        setDirtyRepos(dirty);
        return;
      }
    }
    await launch(args);
  };

  /**
   * Mission mode: the daemon creates the mission and its integration plan and starts the planner in
   * the same request. A mission needs a real project - it runs in that project's repos from the first
   * step - so "Auto" is refused here rather than by the daemon.
   */
  const handleSubmitMission = async (
    description: string,
    project: string,
    choice: MissionAgentChoice,
  ) => {
    if (isBusy) return;
    setDraft({ description, project });
    if (!project || project === AUTO_PROJECT) {
      setError(t("create.needsProject"));
      return;
    }
    setIsBusy(true);
    setError(null);
    try {
      const mission = await bridge.createMission(
        {
          title: missionTitleFrom(description),
          goal: description,
          project,
          agents: toMissionAgents(choice),
        },
        target,
      );
      if (elsewhere) {
        notificationsStore.notifySuccess(
          mission.title,
          t("machines.startedElsewhere", { machine: machineLabel }),
        );
      } else {
        onMissionCreated?.(mission);
      }
      finish();
    } catch (err) {
      setIsBusy(false);
      setError(describeBridgeError(err));
    }
  };

  /** V1 `LaunchWithSync`: a SyncRepo per dirty repo, and CreatePlan waiting behind all of them. */
  const handleSyncRepos = async (policy: SyncRepoPolicy) => {
    if (!pending || !dirtyRepos) return;
    setIsBusy(true);
    const syncJobIds: string[] = [];
    try {
      for (const repo of dirtyRepos) {
        const res = await jobsStore.startJob(
          {
            type: "SyncRepo",
            repoPath: repo.path,
            baseBranch: repo.baseBranch ?? "main",
            untrackedChangesPolicy: policy,
          },
          target,
        );
        syncJobIds.push(res.jobId);
      }
    } catch (err) {
      setIsBusy(false);
      setDirtyRepos(null);
      setError(describeBridgeError(err));
      return;
    }
    await launch(pending, syncJobIds);
  };

  const handleUploadFile = async (file: CreatePlanUpload): Promise<string> => {
    const staged = await bridge.uploadAttachmentBytes(
      file.name,
      file.base64Data,
      uploadSessionId.current,
    );
    uploaded.current = true;
    return staged.path;
  };

  /** V1 `OnMenuAction` → `ChatLauncher.Open(nav, config, BuildAgentPrompt(...))`. */
  const handleContinueInChat = (description: string, project: string) => {
    const prompt = buildCreatePlanAgentPrompt(project, description);
    onClose();
    uiStore.navigate({ appId: "chat" });
    void import("../state/chatStore").then(async ({ chatStore }) => {
      try {
        await chatStore.createSession(NEW_CHAT_TITLE);
        await chatStore.sendMessage(prompt);
      } catch {
        // The chat view reports its own failures through the store's `error`.
      }
    });
  };

  if (!isOpen) return null;

  if (dirtyRepos && pending) {
    return (
      <DirtyRepoDialog
        isOpen
        purpose="createPlan"
        dirtyRepos={dirtyRepos}
        onClose={() => {
          // V1 drops the pending job with the dialog: Cancel means "do not create it".
          setDirtyRepos(null);
          setPending(null);
          onClose();
        }}
        onProceed={() => void launch(pending)}
        onSyncRepos={(policy) => void handleSyncRepos(policy)}
      />
    );
  }

  return (
    <CreatePlanDialog
      isOpen={isOpen}
      onClose={onClose}
      projects={shownProjects.map((p) => p.name)}
      initialProject={draft?.project ?? initialProject}
      initialDescription={draft?.description ?? combine(initialTitle, initialDescription)}
      onSubmit={handleSubmit}
      onSubmitMission={onMissionCreated ? handleSubmitMission : undefined}
      initialMode={initialMode}
      machines={machines.map((m) => ({
        value: m.id,
        label: m.id === "local" ? t("machines.local") : m.host,
        disabled: !m.available,
      }))}
      selectedMachine={machine}
      onMachineChange={handleMachineChange}
      agentOptions={visibleAgents(config).map((agent) => ({ value: agent.id, label: agent.label }))}
      defaultAgentLabel={agentLabel}
      onAddProject={onAddProject}
      addProjectKeepsOpen={addProjectKeepsOpen}
      selectProject={selectProject}
      agentLabel={agentLabel}
      onContinueInChat={handleContinueInChat}
      onUploadFile={handleUploadFile}
      isBusy={isBusy}
      error={error}
    />
  );
};
