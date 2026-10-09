import { invoke } from "@tauri-apps/api/core";
import {
  decodeBase64,
  onReviewActionEvent,
  subscribeReviewAction,
  type EventUnsubscribe,
  type ReviewActionEvent,
  type ReviewActionSession,
} from "./events";
import type {
  CommitBrief,
  CommitDetail,
  DiffTarget,
  FileDiff,
  Graph,
  OpResult,
  PrPrefill,
  RepoCard,
  RepoDetail,
  RepoOp,
  RepoPrs,
} from "../types/git";
import type {
  AddedProjectRepo,
  AgentCostBreakdown,
  Annotation,
  CreateProjectRequest,
  CreatedProject,
  CrossPlanRecommendation,
  DashboardActivity,
  DiscoveredVaultRepo,
  DoctorCheck,
  DraftComment,
  GitHubAccountOption,
  GitHubIssuesPage,
  GitHubRepo,
  InboxProposal,
  CreateMissionRequest,
  BranchPreview,
  GitSettings,
  Machine,
  MachineTarget,
  Mission,
  MissionAction,
  MissionAgents,
  MissionBudget,
  Job,
  JobDetail,
  ModelCatalogStatus,
  OnboardingStatus,
  PlanArtifactContent,
  PlanArtifacts,
  PlanChangesData,
  PlanCommitDetail,
  PlanDetail,
  PlanGitData,
  PlanQuery,
  PlanSummary,
  PrStatus,
  PrSyncReport,
  ProjectAssets,
  ProjectSummary,
  ProvisionReport,
  RecentMergedPr,
  RecentPlanCost,
  RecommendationItem,
  RecommendationState,
  RepoStatus,
  IssueMetadata,
  ReviewActionConditionResult,
  ReviewActionConfig,
  RevisionResult,
  ServiceHealth,
  ServiceInfo,
  ShippedFeatureDay,
  StartJobArgs,
  StartJobResponse,
  SubscribeOutcome,
  SweepReport,
  TendrilConfig,
  VaultCatalog,
  VaultExportRequest,
  VaultImportRequest,
  VaultPrResult,
  VaultResult,
  VaultStatus,
  VerificationReport,
  VerificationStatus,
  VersionInfo,
  DirectoryListing,
} from "../types/api";
import type { ChatAttachment } from "../types/chat";
import type { ManagerActivity } from "../state/managerActivity";

/** A public-API key as the daemon lists it: never the secret, which only creation returns. */
export interface ApiKeyRecord {
  id: string;
  name: string;
  scope: "read" | "write";
  prefix: string;
  created: string;
}
import type {
  DiscoveredRepoAsset,
  ProjectMemoryEntry,
  ManagerTask,
  ProjectDocker,
  TelegramStatus,
  EngineChoice,
  GlobalEngine,
  MissionEvidence,
  ProjectEngineRoles,
  ProjectMemoryFile,
  RepoAssetKind,
} from "../types/projectAssets";
import { encodeBase64, isTauri } from "../utils/tauri";
import { i18n } from "../i18n";

export type { ReviewActionSession } from "./events";

export interface ReviewActionRunOptions {
  planId?: string;
  worktree?: string;
  /** One chunk of raw terminal output: escapes, bare carriage returns and partial sequences included. */
  onChunk?: (bytes: Uint8Array) => void;
  /** The process's exit message, after which no more output arrives. */
  onEnd?: (message: string) => void;
  onError?: (err: unknown) => void;
}

/** A running review action, and the three things a terminal view needs to do to it. */
export interface ReviewActionRun {
  session: ReviewActionSession;
  /** Sends keystrokes. A string is sent as UTF-8. */
  sendInput(data: string | Uint8Array): Promise<void>;
  resize(rows: number, cols: number): Promise<void>;
  /**
   * Stops watching the output. Does not stop the process — the app it started has to keep serving the
   * preview that replaces the terminal.
   */
  close(): Promise<void>;
}

function reviewActionUrl(projectName: string, actionName: string, endpoint: string): string {
  return (
    `/api/projects/${encodeURIComponent(projectName)}` +
    `/review-actions/${encodeURIComponent(actionName)}/${endpoint}`
  );
}

/**
 * Desktop transport: the native side reads the stream and re-emits it, because `invoke` cannot stream
 * and the daemon's route is bearer-authenticated with a secret the webview never sees.
 *
 * The listener is registered before the invoke and frames are held until the session id comes back,
 * because the process can write before the invoke's return value has crossed the boundary. Frames
 * belonging to other sessions are dropped on the replay, once there is an id to compare them to.
 */
async function startReviewActionViaTauri(
  projectName: string,
  actionName: string,
  options: ReviewActionRunOptions,
): Promise<ReviewActionRun> {
  let session: ReviewActionSession | null = null;
  const pending: ReviewActionEvent[] = [];

  const deliver = (frame: ReviewActionEvent) => {
    if (frame.event === "end") {
      options.onEnd?.(frame.data);
      return;
    }
    if (frame.event === "log") {
      try {
        options.onChunk?.(decodeBase64(frame.data));
      } catch (err) {
        options.onError?.(err);
      }
    }
  };

  const unlisten = await onReviewActionEvent((frame) => {
    if (!session) {
      pending.push(frame);
      return;
    }
    if (frame.sessionId === session.sessionId) {
      deliver(frame);
    }
  });

  try {
    session = await invoke<ReviewActionSession>("cmd_execute_review_action", {
      projectName,
      actionName,
      planId: options.planId,
      worktree: options.worktree,
    });
  } catch (err) {
    unlisten();
    throw err;
  }

  for (const frame of pending) {
    if (frame.sessionId === session.sessionId) {
      deliver(frame);
    }
  }
  pending.length = 0;

  const sessionId = session.sessionId;
  return {
    session,
    async sendInput(data) {
      await invoke<void>("cmd_send_review_action_input", {
        projectName,
        actionName,
        sessionId,
        data: encodeBase64(data),
      });
    },
    async resize(rows, cols) {
      await invoke<void>("cmd_resize_review_action", {
        projectName,
        actionName,
        sessionId,
        rows,
        cols,
      });
    },
    async close() {
      unlisten();
      await invoke<boolean>("cmd_close_review_action", { sessionId });
    },
  };
}

/** Browser transport: the webview reads the SSE stream itself, same-origin through the dev proxy. */
async function startReviewActionViaHttp(
  projectName: string,
  actionName: string,
  options: ReviewActionRunOptions,
): Promise<ReviewActionRun> {
  let unsubscribe: EventUnsubscribe = () => {};

  const session = await new Promise<ReviewActionSession>((resolve, reject) => {
    let settled = false;
    unsubscribe = subscribeReviewAction("", projectName, actionName, {
      planId: options.planId,
      worktree: options.worktree,
      onSession: (announced) => {
        settled = true;
        resolve(announced);
      },
      onChunk: (bytes) => options.onChunk?.(bytes),
      onEnd: (message) => {
        // A command that fails to start exits before announcing anything; that end message is the
        // only explanation the caller will get.
        if (!settled) {
          settled = true;
          reject(new Error(message));
          return;
        }
        options.onEnd?.(message);
      },
      onError: (err) => {
        if (!settled) {
          settled = true;
          reject(err);
          return;
        }
        options.onError?.(err);
      },
    });
  }).catch((err) => {
    unsubscribe();
    throw err;
  });

  const post = async (endpoint: string, body: Record<string, unknown>) => {
    const res = await fetch(reviewActionUrl(projectName, actionName, endpoint), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ sessionId: session.sessionId, ...body }),
    });
    if (!res.ok) {
      const detail = await res.text().catch(() => "");
      throw new Error(
        i18n.t("common:errors.reviewActionFailed", { endpoint, status: res.status, detail }),
      );
    }
  };

  return {
    session,
    async sendInput(data) {
      await post("input", { data: encodeBase64(data) });
    },
    async resize(rows, cols) {
      await post("resize", { rows, cols });
    },
    async close() {
      unsubscribe();
    },
  };
}

/**
 * A call that prefers Tauri IPC and falls back to the daemon over HTTP.
 *
 * IPC is the real path: the daemon's bearer secret is read from `.master` on the native side and never
 * enters the webview, so only the Rust command can authenticate. The `fetch` is for running the UI in
 * a plain browser during development, where the dev server proxies `/api`.
 */
async function invokeOrFetch<T>(
  command: string,
  args: Record<string, unknown>,
  path: string,
  init?: RequestInit,
): Promise<T> {
  // Under Tauri the command's own rejection is the answer. Falling back to `fetch(path)` there cannot
  // work - the webview's origin serves the app's assets, so the "response" is `index.html` - and it
  // replaced the daemon's real reason with WebKit's JSON parse error ("The string did not match the
  // expected pattern").
  if (isTauri()) {
    return invoke<T>(command, args);
  }

  /* `Headers` rather than an object spread: `HeadersInit` also allows an array of pairs, and
     spreading one of those would turn it into numeric keys. */
  const headers = new Headers(init?.headers);
  if (!headers.has("Content-Type")) headers.set("Content-Type", "application/json");

  const res = await fetch(path, { ...init, headers });
  if (!res.ok) {
    /* A failed vault result answers 500 carrying the message the dialogs show, so it is a value
       rather than an error — see `vault_request` in src-tauri. */
    const body = await res.json().catch(() => null);
    if (body && typeof body === "object" && (body as { success?: unknown }).success === false) {
      return body as T;
    }
    throw new Error(i18n.t("common:errors.requestFailed", { path, status: res.status }));
  }
  return res.json() as Promise<T>;
}

/** `default` is the id the service resolves to the primary vault. */
function vaultPath(vaultId: string | undefined, suffix = ""): string {
  return `/api/vaults/${encodeURIComponent(vaultId?.trim() || "default")}${suffix}`;
}

export const tauriClient = {
  async checkServiceHealth(this: void): Promise<ServiceHealth> {
    return invoke<ServiceHealth>("cmd_check_service_health");
  },

  async getServiceInfo(this: void): Promise<ServiceInfo> {
    return invoke<ServiceInfo>("cmd_get_service_info");
  },

  async getServiceLogs(this: void, lines?: number): Promise<string[]> {
    return invoke<string[]>("cmd_get_service_logs", { lines });
  },

  async restartService(this: void): Promise<ServiceInfo> {
    return invoke<ServiceInfo>("cmd_restart_service");
  },

  async repairService(this: void): Promise<string> {
    return invoke<string>("cmd_repair_service");
  },

  async switchServiceMode(this: void, mode: string): Promise<ServiceInfo> {
    return invoke<ServiceInfo>("cmd_switch_service_mode", { mode });
  },

  /**
   * Installs the bundled `tendril` and `opencode` sidecars into `<tendril home>/bin` and registers
   * the daemon to start with the session. The app already does this on first run; this is the retry
   * for a machine that refused it then.
   */
  async installService(this: void): Promise<ProvisionReport> {
    return invoke<ProvisionReport>("cmd_install_service");
  },

  /** Stops the daemon starting at login. The installed binaries stay where they are. */
  async uninstallServiceAutostart(this: void): Promise<string> {
    return invoke<string>("cmd_uninstall_service_autostart");
  },

  async listPlans(this: void, query?: PlanQuery): Promise<PlanSummary[]> {
    return invoke<PlanSummary[]>("cmd_list_plans", { query });
  },

  async getPlan(this: void, id: string): Promise<PlanDetail> {
    return invoke<PlanDetail>("cmd_get_plan", { id });
  },

  async updatePlanField(
    this: void,
    id: string,
    field: string,
    value: string,
    allowFailed?: boolean,
  ): Promise<void> {
    return invoke<void>("cmd_update_plan_field", {
      id,
      field,
      value,
      allowFailed,
    });
  },

  /**
   * Permanently delete a plan folder and its database row. Rejects with a
   * `CONFLICT` bridge error while a job still holds the plan.
   */
  async deletePlan(this: void, id: string): Promise<void> {
    return invoke<void>("cmd_delete_plan", { id });
  },

  /**
   * Send a plan back to Draft and remove its worktrees. Rejects with a
   * `CONFLICT` bridge error for Completed/Skipped plans and for running ones.
   */
  async resetPlan(this: void, id: string): Promise<void> {
    return invoke<void>("cmd_reset_plan", { id });
  },

  /** Uncommitted-change status of each repo the plan targets. */
  async getRepoStatus(this: void, id: string): Promise<RepoStatus[]> {
    return invoke<RepoStatus[]>("cmd_get_repo_status", { id });
  },

  /**
   * Uncommitted-change status of each of a project's repos, with the base branch each syncs to - the
   * create-plan dirty-repo preflight (V1 `UsePreflightCheck(project)`).
   */
  async getProjectRepoStatus(
    this: void,
    projectName: string,
    target?: MachineTarget,
  ): Promise<RepoStatus[]> {
    return invoke<RepoStatus[]>("cmd_get_project_repo_status", { projectName, target });
  },

  /** A project's GitHub labels and assignable users (`gh`, cached daemon-side), for Create Issue. */
  async getProjectIssueMetadata(this: void, projectName: string): Promise<IssueMetadata> {
    return invoke<IssueMetadata>("cmd_get_project_issue_metadata", { projectName });
  },

  /**
   * The plan's worktrees, its commits grouped under them, and the reachability
   * verdict for the commits no worktree accounts for — the Git tab's data.
   */
  async getPlanGit(this: void, id: string): Promise<PlanGitData> {
    return invoke<PlanGitData>("cmd_get_plan_git", { id });
  },

  async getPlanChanges(this: void, id: string): Promise<PlanChangesData> {
    return invokeOrFetch<PlanChangesData>(
      "cmd_get_plan_changes",
      { id },
      `/api/plans/${encodeURIComponent(id)}/changes`,
    );
  },

  async getPlanSummary(this: void, id: string): Promise<string | null> {
    const res = await invokeOrFetch<{ summary: string | null } | string | null>(
      "cmd_get_plan_summary",
      { id },
      `/api/plans/${encodeURIComponent(id)}/summary`,
    );
    if (res && typeof res === "object" && "summary" in res) {
      return res.summary;
    }
    return (res as string | null) ?? null;
  },

  async getPlanArtifacts(this: void, id: string): Promise<PlanArtifacts> {
    return invokeOrFetch<PlanArtifacts>(
      "cmd_get_plan_artifacts",
      { id },
      `/api/plans/${encodeURIComponent(id)}/artifacts`,
    );
  },

  /**
   * One artifact's text for the Review app's artifact sheet, by the absolute path
   * `getPlanArtifacts` listed. The daemon refuses a path that does not resolve inside the plan's
   * `Artifacts/` folder (`VALIDATION_ERROR`), and answers `binary` / `tooLarge` rather than text for
   * a file there is nothing to show of. Images go through `getLocalFilePreview` instead.
   */
  async getPlanArtifactContent(this: void, id: string, path: string): Promise<PlanArtifactContent> {
    return invokeOrFetch<PlanArtifactContent>(
      "cmd_get_plan_artifact_content",
      { id, path },
      `/api/plans/${encodeURIComponent(id)}/artifacts/content?path=${encodeURIComponent(path)}`,
    );
  },

  /**
   * One of a plan's commits for the commit detail sheet: its subject, files and per-file patch, from
   * the first of the plan's repos (then worktrees) that holds it. `null` when none does any more.
   * Read natively - see `commands::plan_files` in the Tauri crate.
   */
  async getPlanCommit(this: void, id: string, hash: string): Promise<PlanCommitDetail | null> {
    return invoke<PlanCommitDetail | null>("cmd_get_plan_commit", { id, hash });
  },

  /**
   * A local file markdown links to, for the file sheet. Refused (`VALIDATION_ERROR`) unless it
   * resolves inside the plan's folder or one of the repos the plan targets - or, with no plan (an
   * Inbox issue), inside a configured project's repo; answers `binary` /
   * `tooLarge` the way the artifact read does. Images go through `getLocalFilePreview` instead.
   */
  async getPlanFileContent(
    this: void,
    id: string | null | undefined,
    path: string,
  ): Promise<PlanArtifactContent> {
    return invoke<PlanArtifactContent>("cmd_get_plan_file_content", { id: id ?? null, path });
  },

  async getRevision(this: void, id: string, number?: number): Promise<string> {
    return invoke<string>("cmd_get_revision", { id, number });
  },

  async writeRevision(this: void, id: string, content: string): Promise<RevisionResult> {
    return invoke<RevisionResult>("cmd_write_revision", { id, content });
  },

  /**
   * Overwrite the plan's newest revision in place, keeping its number.
   *
   * The write answering a plan question needs, and deliberately not `writeRevision`, which appends.
   * V1 routes answers through `IPlanReaderService.UpdateLatestRevision` on the stated grounds that
   * "answering a question is not a new revision of the plan, it is filling in a blank the plan left".
   * An append would claim the agent produced a new plan, and it would inflate `revisionCount`, which
   * `executeGuards.unfoldedAnswerCount` reads as `revisionCount === 1` — so one answer would switch
   * that guard off. The returned `revision` is the number that did *not* move.
   */
  async updateLatestRevision(this: void, id: string, content: string): Promise<RevisionResult> {
    return invoke<RevisionResult>("cmd_update_latest_revision", { id, content });
  },

  /**
   * Every inline diff comment drafted against a plan, across all revision pairs.
   *
   * The mutations below all return the plan's new full list, so a caller replaces its state from
   * the response instead of guessing at the outcome.
   */
  async listDiffComments(this: void, planId: string): Promise<DraftComment[]> {
    return invoke<DraftComment[]>("cmd_list_diff_comments", { planId });
  },

  async upsertDiffComment(
    this: void,
    planId: string,
    comment: DraftComment,
  ): Promise<DraftComment[]> {
    return invoke<DraftComment[]>("cmd_upsert_diff_comment", { planId, comment });
  },

  async deleteDiffComment(
    this: void,
    planId: string,
    filePath: string,
    changeKey: string,
  ): Promise<DraftComment[]> {
    return invoke<DraftComment[]>("cmd_delete_diff_comment", { planId, filePath, changeKey });
  },

  async clearDiffComments(this: void, planId: string): Promise<void> {
    return invoke<void>("cmd_clear_diff_comments", { planId });
  },

  /**
   * Every draft annotation left on a plan's revision markdown.
   *
   * Same contract as the diff comments above: the mutations return the plan's new full list.
   */
  async listAnnotations(this: void, planId: string): Promise<Annotation[]> {
    return invoke<Annotation[]>("cmd_list_annotations", { planId });
  },

  async upsertAnnotation(
    this: void,
    planId: string,
    annotation: Annotation,
  ): Promise<Annotation[]> {
    return invoke<Annotation[]>("cmd_upsert_annotation", { planId, annotation });
  },

  async deleteAnnotation(this: void, planId: string, annotationId: string): Promise<Annotation[]> {
    return invoke<Annotation[]>("cmd_delete_annotation", { planId, annotationId });
  },

  async clearAnnotations(this: void, planId: string): Promise<void> {
    return invoke<void>("cmd_clear_annotations", { planId });
  },

  /** Markdown of one `<planFolder>/Verification/<name>.md` report. */
  async getVerificationReport(
    this: void,
    planId: string,
    name: string,
  ): Promise<VerificationReport> {
    return invoke<VerificationReport>("cmd_get_verification_report", {
      planId,
      name,
    });
  },

  /** Every verification report that exists on disk for a plan. */
  async listVerificationReports(this: void, planId: string): Promise<VerificationReport[]> {
    return invoke<VerificationReport[]>("cmd_list_verification_reports", {
      planId,
    });
  },

  async setVerificationStatus(
    this: void,
    planId: string,
    name: string,
    status: VerificationStatus,
  ): Promise<void> {
    return invoke<void>("cmd_set_verification_status", {
      planId,
      name,
      status,
    });
  },

  async listRecommendations(this: void, planId: string): Promise<RecommendationItem[]> {
    return invoke<RecommendationItem[]>("cmd_list_recommendations", { planId });
  },

  async listCrossPlanRecommendations(
    this: void,
    project?: string,
    state?: string,
  ): Promise<CrossPlanRecommendation[]> {
    try {
      return await invoke<CrossPlanRecommendation[]>("cmd_list_all_recommendations", {
        project,
        state,
      });
    } catch {
      // Cross-plan projection fallback: gather from completed plans
      const plans = await bridge.listPlans();
      const relevantPlans = plans.filter((p) => p.state === "Completed" || !state);
      const results: CrossPlanRecommendation[] = [];
      await Promise.all(
        relevantPlans.map(async (plan) => {
          try {
            const recs = await bridge.listRecommendations(plan.id);
            for (const r of recs) {
              const rState = r.state || "Pending";
              if (state && rState !== state) continue;
              if (project && plan.project !== project) continue;
              results.push({
                planId: plan.id,
                planTitle: plan.title,
                project: plan.project,
                sourcePlanStatus: plan.state,
                title: r.title,
                description: r.description,
                impact: r.impact,
                state: r.state,
                declineReason: r.declineReason,
                notes: r.notes,
              });
            }
          } catch {
            // Ignore per-plan failure
          }
        }),
      );
      return results;
    }
  },

  /**
   * `declineReason` and `notes` are separate fields, not one field reused: a
   * decline reason is why the recommendation was rejected, a note is why it was
   * accepted. Pass `notes` with `AcceptedWithNotes` and `declineReason` with
   * `Declined`.
   */
  async setRecommendationState(
    this: void,
    planId: string,
    title: string,
    state: RecommendationState,
    declineReason?: string,
    notes?: string,
  ): Promise<void> {
    return invoke<void>("cmd_set_recommendation_state", {
      planId,
      title,
      state,
      declineReason,
      notes,
    });
  },

  async listJobs(this: void, status?: string, limit?: number): Promise<Job[]> {
    return invoke<Job[]>("cmd_list_jobs", { status, limit });
  },

  async getJob(this: void, id: string): Promise<JobDetail> {
    return invoke<JobDetail>("cmd_get_job", { id });
  },

  /**
   * `target: "local"` starts the job on this machine's daemon even while the app is connected to a
   * remote one (the create-plan machine picker); omitted, it goes where everything else goes.
   */
  async startJob(
    this: void,
    args: StartJobArgs,
    target?: MachineTarget,
  ): Promise<StartJobResponse> {
    return invoke<StartJobResponse>("cmd_start_job", { args, target });
  },

  /** The machines a plan can be created on; empty unless connected to a remote server. */
  async listMachines(this: void): Promise<Machine[]> {
    return invoke<Machine[]>("cmd_list_machines");
  },

  async cancelJob(this: void, id: string, message?: string): Promise<void> {
    return invoke<void>("cmd_cancel_job", { id, message });
  },

  /** Drops a job from the list and the database. The daemon keeps its log artifacts. */
  async deleteJob(this: void, id: string): Promise<void> {
    return invoke<void>("cmd_delete_job", { id });
  },

  /** Promotes a blocked or queued job past its gates so it runs next. */
  async forceStartJob(this: void, id: string): Promise<void> {
    return invoke<void>("cmd_force_start_job", { id });
  },

  /**
   * V1's Rerun (`RerunJobDialog.cs`): the daemon deletes the finished job and starts it again from its
   * original args, with `feedback` folded in - a `RetryPlan` change request, `UpdatePlan`
   * instructions, or an `ExecutePlan` of the plan a `CreatePlan` produced. Answers the new job.
   */
  async rerunJob(this: void, id: string, feedback?: string): Promise<StartJobResponse> {
    return invoke<StartJobResponse>("cmd_rerun_job", { id, feedback });
  },

  /**
   * V1's Report Bug (`ReportBugDialog.cs` over `BugReportService`): the job's logs, its plan and a
   * sanitized config go up as a **public** GitHub issue, through `tendril report-bug --submit`.
   * Answers the issue's URL.
   */
  async reportJobBug(
    this: void,
    id: string,
    description: string,
    githubUser?: string,
  ): Promise<string> {
    return invoke<string>("cmd_report_job_bug", { id, description, githubUser });
  },

  /**
   * Bulk-clears finished jobs by scope, answering how many rows went.
   *
   * The scope is not validated here. The daemon filters to the terminal statuses before it reads a row,
   * so a Running or Queued job cannot be cleared through any caller, and it answers a `400` naming the
   * scopes it accepts — a refusal worth showing rather than pre-empting with a second list that could
   * fall out of step with it.
   */
  async clearJobs(this: void, status: string): Promise<number> {
    return invoke<number>("cmd_clear_jobs", { status });
  },

  async listProjects(this: void, target?: MachineTarget): Promise<ProjectSummary[]> {
    return invoke<ProjectSummary[]>("cmd_list_projects", { target });
  },

  async listPullRequests(this: void): Promise<PrStatus[]> {
    return invoke<PrStatus[]>("cmd_list_pull_requests");
  },

  /** Rejects with code `PR_SYNC_IN_PROGRESS` when the daemon is already reconciling. */
  async syncPullRequests(this: void): Promise<PrSyncReport> {
    return invoke<PrSyncReport>("cmd_sync_pull_requests");
  },

  async getProjectReviewActions(this: void, projectName: string): Promise<ReviewActionConfig[]> {
    try {
      const projects = await bridge.listProjects();
      const proj = projects.find((p) => p.name.toLowerCase() === projectName.toLowerCase());
      return proj?.reviewActions ?? [];
    } catch {
      return [];
    }
  },

  /**
   * Whether each of the project's review actions has its condition met for `planId`, decided by the
   * daemon against the plan folder. `ReviewActionsBarView` disables a button on `notMet` and says why
   * on hover; see `ReviewActionConditionResult`.
   */
  async getReviewActionConditions(
    this: void,
    projectName: string,
    planId: string,
  ): Promise<ReviewActionConditionResult[]> {
    return invoke<ReviewActionConditionResult[]>("cmd_get_review_action_conditions", {
      projectName,
      planId,
    });
  },

  /**
   * Starts a review action and streams its terminal output.
   *
   * Resolves once the daemon has announced the session, which is what the input and resize routes are
   * keyed by; output can start arriving before that, so both transports buffer rather than drop it.
   */
  async startReviewAction(
    this: void,
    projectName: string,
    actionName: string,
    options: ReviewActionRunOptions = {},
  ): Promise<ReviewActionRun> {
    return isTauri()
      ? startReviewActionViaTauri(projectName, actionName, options)
      : startReviewActionViaHttp(projectName, actionName, options);
  },

  /**
   * Starts a review action without watching its output, returning the session it announced.
   *
   * The stream is left open and unread: the process is a dev server that has to keep serving after
   * the caller has stopped listening.
   */
  async executeReviewAction(
    this: void,
    projectName: string,
    actionName: string,
    planId?: string,
    worktree?: string,
  ): Promise<ReviewActionSession> {
    const run = await bridge.startReviewAction(projectName, actionName, { planId, worktree });
    return run.session;
  },

  /**
   * Creates a project, cloning any remote among `repos` first. The returned project's `repos` are
   * the resolved local paths - the only place the caller can learn where a URL was cloned to.
   */
  async createProject(this: void, request: CreateProjectRequest): Promise<CreatedProject> {
    return invoke<CreatedProject>("cmd_create_project", { request });
  },

  /**
   * Adds one repository to an existing project, cloning it first when `path` is a remote URL.
   *
   * `putConfig("projects", ...)` cannot stand in for this. It merges and saves whatever it is given,
   * so a URL added that way is what lands in `config.yaml` - persisting any credential embedded in
   * it, and leaving a repo entry the daemon skips, because a working directory has to be a directory.
   * Only this route clones. The returned `path` is what was stored, which is the clone's directory
   * for a remote and the typed path for a local one.
   */
  async addProjectRepo(this: void, projectName: string, path: string): Promise<AddedProjectRepo> {
    return invoke<AddedProjectRepo>("cmd_add_project_repo", { projectName, path });
  },

  /**
   * Renames a project, cascading the new name into its plans and database rows.
   *
   * `putConfig("projects", ...)` cannot stand in for this either. That merges the sequence by name,
   * so a renamed entry matches no existing project and is appended next to the original - leaving
   * two projects where there was one. Only this route renames, and only this route rewrites the
   * plans and the Plans/Jobs/Recommendations rows that name the old project.
   *
   * Rejects with `RENAME_PROJECT_FAILED` carrying the daemon's status: 400 for an empty name, 404
   * when the project is gone, 409 when the target name is already taken.
   *
   * Resolves to the new name rather than to the project. The route answers the whole stored
   * `ProjectConfig` - the unprojected wire shape, whose `repos` are objects rather than the name
   * strings {@link ProjectSummary} carries - so typing it as a `ProjectSummary` would be wrong, and
   * projecting it here would duplicate `cmd_list_projects`'s mapping for a value whose only use is
   * to re-read the list. The name is what a caller needs to reselect the project it just renamed.
   */
  async renameProject(this: void, name: string, newName: string): Promise<string> {
    const renamed = await invoke<{ name?: string }>("cmd_rename_project", { name, newName });
    /* Trusting the daemon's echo rather than `newName`: it trims before it stores, so the stored
       name is the one that will match on the next read. */
    return renamed?.name ?? newName;
  },

  /**
   * Removes a project from `config.yaml`.
   *
   * `putConfig("projects", ...)` cannot do this at all: the merge reads an omitted project as
   * unchanged rather than deleted, so a project can only be removed by the route that removes it.
   *
   * Removes the config entry and nothing else - the project's plans, its Plans/Jobs/Recommendations
   * rows and any repository the daemon cloned for it all stay on disk. Any caller must say so
   * before it asks the user to confirm.
   *
   * Named `removeProject` rather than `deleteProject`, which is what it was called while it was the
   * only one of the two. The mismatch was the bug: a method called *delete* that deletes nothing sat
   * behind a button called "Delete Project", and the copy explaining that nothing is deleted was the
   * only thing standing between an operator and the wrong expectation. See
   * {@link bridge.deleteProjectData} for the one that does delete.
   */
  async removeProject(this: void, name: string): Promise<void> {
    await invoke<unknown>("cmd_remove_project", { name });
  },

  /**
   * Deletes a project **and its data** (`DELETE /api/projects/:name/data`).
   *
   * The destructive counterpart to {@link bridge.removeProject}, and a separate route rather than a
   * flag on that one so that the irreversible call cannot be reached by getting a boolean wrong.
   *
   * The daemon removes, in order: every plan folder naming the project (worktrees cleaned first),
   * `<TENDRIL_HOME>/Projects/<name>/` with the clones, skills, MCP definitions and memories inside
   * it, the project's rows in `Plans`, `Jobs` and `Recommendations`, and last the `config.yaml`
   * entry. Job logs under `Logs/Jobs/` are keyed by job id rather than by project and are kept,
   * which the dialog says.
   *
   * 409 while a job of the project is still running, which is the daemon's refusal to delete a
   * worktree out from under a live agent, and a message the caller should show rather than retry.
   */
  async deleteProjectData(this: void, name: string): Promise<void> {
    await invoke<unknown>("cmd_delete_project_data", { name });
  },

  async getConfig(this: void): Promise<TendrilConfig> {
    return invoke<TendrilConfig>("cmd_get_config");
  },

  /** Merges a single top-level key into `config.yaml`, leaving every other key untouched. */
  async putConfig(this: void, key: string, value: unknown): Promise<void> {
    return invoke<void>("cmd_put_config", { key, value });
  },

  /**
   * Subdirectories of `path` on the *daemon's* host (its home directory when omitted). The folder
   * browser uses this rather than the native picker whenever the UI is not the desktop app, since a
   * remote client's own disk is the wrong one to pick a repository from.
   */
  async listDirectories(this: void, path?: string, showHidden = false): Promise<DirectoryListing> {
    const query = new URLSearchParams();
    if (path) query.set("path", path);
    if (showHidden) query.set("showHidden", "true");
    const qs = query.toString();
    return invokeOrFetch<DirectoryListing>(
      "cmd_list_directories",
      { path: path ?? null, showHidden },
      `/api/fs/directories${qs ? `?${qs}` : ""}`,
    );
  },

  async getOnboardingStatus(this: void): Promise<OnboardingStatus> {
    return invoke<OnboardingStatus>("cmd_get_onboarding_status");
  },

  async completeOnboarding(this: void): Promise<void> {
    return invoke<void>("cmd_complete_onboarding");
  },

  async dismissOnboarding(this: void): Promise<void> {
    return invoke<void>("cmd_dismiss_onboarding");
  },

  async subscribeNewsletter(this: void, email: string): Promise<SubscribeOutcome> {
    return invoke<SubscribeOutcome>("cmd_subscribe_newsletter", { email });
  },

  async runDoctor(this: void): Promise<DoctorCheck[]> {
    return invoke<DoctorCheck[]>("cmd_run_doctor");
  },

  async getModelsStatus(this: void): Promise<ModelCatalogStatus> {
    return invoke<ModelCatalogStatus>("cmd_get_models_status");
  },

  async refreshModels(this: void): Promise<ModelCatalogStatus> {
    return invoke<ModelCatalogStatus>("cmd_refresh_models");
  },

  async getVersionInfo(this: void): Promise<VersionInfo> {
    return invoke<VersionInfo>("cmd_get_version_info");
  },

  async checkVersionNow(this: void): Promise<VersionInfo> {
    return invoke<VersionInfo>("cmd_check_version_now");
  },

  async saveUiState(this: void, key: string, value: string): Promise<void> {
    return invoke<void>("cmd_save_ui_state", { key, value });
  },

  async loadUiState(this: void, key: string): Promise<string | null> {
    return invoke<string | null>("cmd_load_ui_state", { key });
  },

  /**
   * Monthly rollups, the daily series and the month's projection in one call. The projection is
   * computed by the daemon, not here, so there is exactly one implementation of it.
   */
  async getDashboardActivity(this: void, months?: number): Promise<DashboardActivity> {
    return invoke<DashboardActivity>("cmd_get_dashboard_activity", { months });
  },

  async getShippedFeatures(this: void, days?: number): Promise<ShippedFeatureDay[]> {
    return invoke<ShippedFeatureDay[]>("cmd_get_shipped_features", { days });
  },

  async getRecentMergedPrs(this: void, limit?: number): Promise<RecentMergedPr[]> {
    return invoke<RecentMergedPr[]>("cmd_get_recent_merged_prs", { limit });
  },

  async getRecentPlanCosts(this: void, days?: number): Promise<RecentPlanCost[]> {
    return invoke<RecentPlanCost[]>("cmd_get_recent_plan_costs", { days });
  },

  async getAgentCostBreakdown(this: void, days?: number): Promise<AgentCostBreakdown[]> {
    return invoke<AgentCostBreakdown[]>("cmd_get_agent_cost_breakdown", { days });
  },

  /* --- Project memory & repo assets ---------------------------------------------------------------
     `<TENDRIL_HOME>/Projects/<Project>/Memory/*.md` (V1's `ProjectMemoryTableView` and
     `EditProjectMemorySheet`) and `ImportRepoAssetsDialog`'s scan/import. The daemon owns every path:
     a memory file is addressed by its bare name, and a repo is re-scanned on import rather than
     trusted from the client. */

  /* --- Public API keys ---------------------------------------------------------------------------
     `/api/api-keys`. The secret is in the create response only; the list never carries it. */

  async listApiKeys(this: void): Promise<ApiKeyRecord[]> {
    const res = await invokeOrFetch<{ keys: ApiKeyRecord[] }>("cmd_list_api_keys", {}, "/api/api-keys");
    return res.keys ?? [];
  },

  async createApiKey(this: void, name: string, write: boolean): Promise<{ key: string; record: ApiKeyRecord }> {
    return invokeOrFetch<{ key: string; record: ApiKeyRecord }>(
      "cmd_create_api_key",
      { name, write },
      "/api/api-keys",
      { method: "POST", body: JSON.stringify({ name, write }) },
    );
  },

  async revokeApiKey(this: void, id: string): Promise<void> {
    await invokeOrFetch<{ ok: boolean }>(
      "cmd_revoke_api_key",
      { id },
      `/api/api-keys/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
  },

  /* --- Git page ----------------------------------------------------------------------------------
     `/api/git/**`. A repository is only ever named by the opaque id the daemon gave it. */

  async gitRepos(this: void): Promise<RepoCard[]> {
    const res = await invokeOrFetch<{ repos: RepoCard[] }>("cmd_git_repos", {}, "/api/git/repos");
    return res.repos ?? [];
  },

  /** Open pull requests per repository id, loaded apart from the cards because `gh` is slow. */
  async gitPrs(this: void): Promise<Record<string, RepoPrs>> {
    const res = await invokeOrFetch<{ repos: Record<string, RepoPrs> }>("cmd_git_prs", {}, "/api/git/prs");
    return res.repos ?? {};
  },

  async gitRepo(this: void, id: string): Promise<RepoDetail> {
    return invokeOrFetch<RepoDetail>("cmd_git_repo", { id }, `/api/git/repos/${encodeURIComponent(id)}`);
  },

  async gitGraph(this: void, id: string, limit: number, branch?: string): Promise<Graph> {
    const query = `limit=${limit}${branch ? `&branch=${encodeURIComponent(branch)}` : ""}`;
    return invokeOrFetch<Graph>(
      "cmd_git_graph",
      { id, limit, branch: branch ?? null },
      `/api/git/repos/${encodeURIComponent(id)}/graph?${query}`,
    );
  },

  async gitCommit(this: void, id: string, hash: string): Promise<CommitDetail> {
    return invokeOrFetch<CommitDetail>(
      "cmd_git_commit",
      { id, hash },
      `/api/git/repos/${encodeURIComponent(id)}/commits/${encodeURIComponent(hash)}`,
    );
  },

  async gitDiff(this: void, id: string, target: DiffTarget, path: string, origPath?: string): Promise<FileDiff> {
    const request = { target, path, origPath };
    return invokeOrFetch<FileDiff>(
      "cmd_git_diff",
      { id, request },
      `/api/git/repos/${encodeURIComponent(id)}/diff`,
      { method: "POST", body: JSON.stringify(request) },
    );
  },

  async gitHistory(this: void, id: string, path: string, limit = 100): Promise<CommitBrief[]> {
    const res = await invokeOrFetch<{ commits: CommitBrief[] }>(
      "cmd_git_history",
      { id, path, limit },
      `/api/git/repos/${encodeURIComponent(id)}/history?path=${encodeURIComponent(path)}&limit=${limit}`,
    );
    return res.commits ?? [];
  },

  /** Runs one operation. A stop on conflicts is a resolved `OpResult` with `outcome.ok === false`. */
  async gitOp(this: void, id: string, op: RepoOp): Promise<OpResult> {
    return invokeOrFetch<OpResult>(
      "cmd_git_op",
      { id, op },
      `/api/git/repos/${encodeURIComponent(id)}/op`,
      { method: "POST", body: JSON.stringify(op) },
    );
  },

  async gitPrPrefill(this: void, id: string, head: string, base: string): Promise<PrPrefill> {
    return invokeOrFetch<PrPrefill>(
      "cmd_git_pr_prefill",
      { id, head, base },
      `/api/git/repos/${encodeURIComponent(id)}/pr-prefill?head=${encodeURIComponent(head)}&base=${encodeURIComponent(base)}`,
    );
  },

  async gitCreatePr(
    this: void,
    id: string,
    request: { head: string; base: string; title: string; body: string; draft: boolean },
  ): Promise<{ url: string }> {
    return invokeOrFetch<{ url: string }>(
      "cmd_git_create_pr",
      { id, request },
      `/api/git/repos/${encodeURIComponent(id)}/pr`,
      { method: "POST", body: JSON.stringify(request) },
    );
  },

  /** The screenshots and recordings a mission's workers attached to prove their changes, by milestone. */
  async getMissionEvidence(this: void, missionId: string): Promise<MissionEvidence> {
    return invokeOrFetch<MissionEvidence>(
      "cmd_mission_evidence",
      { id: missionId },
      `/api/missions/${encodeURIComponent(missionId)}/evidence`,
    );
  },

  /** Every screenshot and recording attached to any of a project's plans, newest first. */
  async getProjectEvidence(this: void, project: string): Promise<MissionEvidence> {
    return invokeOrFetch<MissionEvidence>(
      "cmd_project_evidence",
      { projectName: project },
      `/api/projects/${encodeURIComponent(project)}/evidence`,
    );
  },

  /** The engines every project uses unless it sets its own, and the rate-limit fallback chain. */
  async getGlobalEngine(this: void): Promise<GlobalEngine> {
    const res = await invokeOrFetch<Partial<GlobalEngine>>("cmd_get_global_engine", {}, "/api/engine");
    return { roles: res.roles ?? {}, fallbacks: res.fallbacks ?? [], limited: res.limited ?? [] };
  },

  /** Sets global roles (`null` clears one) and/or replaces the whole fallback chain. */
  async setGlobalEngine(
    this: void,
    change: { roles?: Record<string, EngineChoice | null>; fallbacks?: EngineChoice[]; applyToAll?: boolean },
  ): Promise<GlobalEngine> {
    const res = await invokeOrFetch<Partial<GlobalEngine>>(
      "cmd_set_global_engine",
      { body: change },
      "/api/engine",
      { method: "PUT", body: JSON.stringify(change) },
    );
    return {
      roles: res.roles ?? {},
      fallbacks: res.fallbacks ?? [],
      limited: res.limited ?? [],
      projectsOverridden: res.projectsOverridden,
    };
  },

  /** The engine each role of a project runs on; a role not listed runs on the default. */
  async getProjectEngine(this: void, projectName: string): Promise<ProjectEngineRoles> {
    const res = await invokeOrFetch<{ roles: ProjectEngineRoles }>(
      "cmd_get_project_engine",
      { projectName },
      `/api/projects/${encodeURIComponent(projectName)}/engine`,
    );
    return res.roles ?? {};
  },

  /** Sets the engine of the named roles; `null` puts a role back on the default. */
  async setProjectEngine(
    this: void,
    projectName: string,
    roles: Record<string, EngineChoice | null>,
  ): Promise<ProjectEngineRoles> {
    const res = await invokeOrFetch<{ roles: ProjectEngineRoles }>(
      "cmd_set_project_engine",
      { projectName, roles },
      `/api/projects/${encodeURIComponent(projectName)}/engine`,
      { method: "PUT", body: JSON.stringify({ roles }) },
    );
    return res.roles ?? {};
  },

  /** Whether each project's manager is mid-turn, and when its conversation last moved. */
  async getManagersStatus(this: void): Promise<ManagerActivity> {
    return invokeOrFetch<ManagerActivity>(
      "cmd_managers_status",
      {},
      "/api/projects/managers",
    );
  },

  /** Project name to the owner of its first repo's `origin` remote (null when there is none). */
  async getProjectOwners(this: void): Promise<Record<string, string | null>> {
    return invokeOrFetch<Record<string, string | null>>("cmd_project_owners", {}, "/api/projects/owners");
  },

  /** The project's Factory Manager chat session (created on first use). */
  async getProjectManager(this: void, projectName: string): Promise<{ id: string; title: string }> {
    return invokeOrFetch<{ id: string; title: string }>(
      "cmd_get_project_manager",
      { projectName },
      `/api/projects/${encodeURIComponent(projectName)}/manager`,
      { method: "POST" },
    );
  },

  /**
   * The Telegram bot: read its status, set it from a BotFather token, unpair the account, send a test
   * message, or remove it. Every action answers with the status afterwards (`test` with `{ sent }`).
   */
  async telegram(
    this: void,
    action: "status" | "set" | "unpair" | "test" | "remove",
    token?: string,
  ): Promise<TelegramStatus> {
    const [path, init]: [string, RequestInit] =
      action === "status"
        ? ["/api/telegram", { method: "GET" }]
        : action === "set"
          ? ["/api/telegram", { method: "PUT", body: JSON.stringify({ token }) }]
          : action === "remove"
            ? ["/api/telegram", { method: "DELETE" }]
            : [`/api/telegram/${action}`, { method: "POST" }];
    return invokeOrFetch<TelegramStatus>("cmd_telegram", { action, token: token ?? null }, path, init);
  },

  /** The tasks the project's manager handed straight to an agent: running, and finished but not cleaned up. */
  async listManagerTasks(this: void, projectName: string): Promise<ManagerTask[]> {
    const body = await invokeOrFetch<{ tasks?: ManagerTask[] }>(
      "cmd_list_manager_tasks",
      { projectName },
      `/api/projects/${encodeURIComponent(projectName)}/manager/tasks`,
    );
    return body.tasks ?? [];
  },

  /** The Docker containers belonging to the project; `available` is false without Docker. */
  async getProjectDocker(this: void, projectName: string): Promise<ProjectDocker> {
    return invokeOrFetch<ProjectDocker>(
      "cmd_project_docker",
      { projectName },
      `/api/projects/${encodeURIComponent(projectName)}/docker`,
    );
  },

  async listProjectMemory(this: void, projectName: string): Promise<ProjectMemoryEntry[]> {
    return invokeOrFetch<ProjectMemoryEntry[]>(
      "cmd_list_project_memory",
      { projectName },
      `/api/projects/${encodeURIComponent(projectName)}/memory`,
    );
  },

  async getProjectMemory(
    this: void,
    projectName: string,
    fileName: string,
  ): Promise<ProjectMemoryFile> {
    return invokeOrFetch<ProjectMemoryFile>(
      "cmd_get_project_memory",
      { projectName, fileName },
      `/api/projects/${encodeURIComponent(projectName)}/memory/${encodeURIComponent(fileName)}`,
    );
  },

  /**
   * Creates or overwrites a memory file; `.md` is appended by the daemon when missing. With
   * `previousFileName` naming a different file the save is a rename, refused with `CONFLICT` when
   * the new name is already taken.
   */
  async saveProjectMemory(
    this: void,
    projectName: string,
    fileName: string,
    content: string,
    previousFileName?: string,
  ): Promise<ProjectMemoryFile> {
    return invokeOrFetch<ProjectMemoryFile>(
      "cmd_put_project_memory",
      { projectName, fileName, content, previousFileName },
      `/api/projects/${encodeURIComponent(projectName)}/memory/${encodeURIComponent(fileName)}`,
      { method: "PUT", body: JSON.stringify({ content, previousFileName }) },
    );
  },

  async deleteProjectMemory(this: void, projectName: string, fileName: string): Promise<void> {
    await invokeOrFetch<unknown>(
      "cmd_delete_project_memory",
      { projectName, fileName },
      `/api/projects/${encodeURIComponent(projectName)}/memory/${encodeURIComponent(fileName)}`,
      { method: "DELETE" },
    );
  },

  /** Resolves `source` (a project repo path, a local folder, or a git URL it clones) and scans it. */
  async scanRepoAssets(
    this: void,
    projectName: string,
    kind: RepoAssetKind,
    source: string,
  ): Promise<DiscoveredRepoAsset[]> {
    const res = await invokeOrFetch<{ items: DiscoveredRepoAsset[] }>(
      "cmd_scan_repo_assets",
      { projectName, kind, source },
      `/api/projects/${encodeURIComponent(projectName)}/repo-assets/scan`,
      { method: "POST", body: JSON.stringify({ kind, source }) },
    );
    return res.items;
  },

  /** Imports the named items into the project (skills are copied under its `Skills` folder). */
  async importRepoAssets(
    this: void,
    projectName: string,
    kind: RepoAssetKind,
    source: string,
    names: string[],
  ): Promise<string[]> {
    const res = await invokeOrFetch<{ imported: string[] }>(
      "cmd_import_repo_assets",
      { projectName, kind, source, names },
      `/api/projects/${encodeURIComponent(projectName)}/repo-assets/import`,
      { method: "POST", body: JSON.stringify({ kind, source, names }) },
    );
    return res.imported;
  },

  /* --- Team Vault ------------------------------------------------------------------------------
     These wrappers are the only place that knows command names and route shapes: the vault
     components take plain data and callbacks. `vaultId` is optional everywhere — omitting it means
     the primary vault, which is what `default` resolves to on the service. */

  async listVaults(this: void): Promise<VaultStatus[]> {
    return invokeOrFetch<VaultStatus[]>("cmd_vault_list", {}, "/api/vaults");
  },

  async getVaultStatus(this: void, vaultId?: string): Promise<VaultStatus> {
    return invokeOrFetch<VaultStatus>("cmd_vault_status", { vaultId }, vaultPath(vaultId));
  },

  async getVaultCatalog(this: void, vaultId?: string): Promise<VaultCatalog> {
    return invokeOrFetch<VaultCatalog>(
      "cmd_vault_catalog",
      { vaultId },
      vaultPath(vaultId, "/catalog"),
    );
  },

  /** The GitHub identities a vault can be created under. Empty means "not signed in". */
  async listGitHubAccounts(this: void): Promise<GitHubAccountOption[]> {
    return invokeOrFetch<GitHubAccountOption[]>(
      "cmd_vault_github_accounts",
      {},
      "/api/vaults/accounts",
    );
  },

  /**
   * Every GitHub repository the daemon's `gh` user can clone, most recently updated first, for the
   * Add Project picker. Rejects with `gh`'s own reason (not installed, not signed in).
   */
  async listGitHubRepos(this: void): Promise<GitHubRepo[]> {
    return invokeOrFetch<GitHubRepo[]>("cmd_list_github_repos", {}, "/api/github/repos");
  },

  /** Repositories on GitHub that look like a Tendril vault, for the connect dialog. */
  async discoverVaults(this: void): Promise<DiscoveredVaultRepo[]> {
    return invokeOrFetch<DiscoveredVaultRepo[]>("cmd_vault_discover", {}, "/api/vaults/discover");
  },

  async createVaultRepo(
    this: void,
    name: string,
    isPrivate: boolean,
    org?: string,
  ): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>(
      "cmd_vault_create",
      { name, isPrivate, org },
      "/api/vaults/create",
      { method: "POST", body: JSON.stringify({ repoName: name, private: isPrivate, org }) },
    );
  },

  async connectVault(this: void, repoUrl: string, name?: string): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>("cmd_vault_connect", { repoUrl, name }, "/api/vaults", {
      method: "POST",
      body: JSON.stringify({ repoUrl, name }),
    });
  },

  /** Forget a vault. The clone is left on disk, so nothing local is lost. */
  async disconnectVault(this: void, vaultId?: string): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>("cmd_vault_disconnect", { vaultId }, vaultPath(vaultId), {
      method: "DELETE",
    });
  },

  async setVaultAlwaysUpToDate(
    this: void,
    alwaysUpToDate: boolean,
    vaultId?: string,
  ): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>(
      "cmd_vault_set_always_up_to_date",
      { vaultId, alwaysUpToDate },
      vaultPath(vaultId),
      { method: "PUT", body: JSON.stringify({ alwaysUpToDate }) },
    );
  },

  async pullVaultLatest(this: void, vaultId?: string): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>("cmd_vault_pull", { vaultId }, vaultPath(vaultId, "/pull"), {
      method: "POST",
    });
  },

  /** What a local project could publish, for the push dialog's asset checklists. */
  async collectProjectAssets(this: void, projectName: string): Promise<ProjectAssets> {
    return invokeOrFetch<ProjectAssets>(
      "cmd_vault_project_assets",
      { projectName },
      `/api/vaults/project-assets/${encodeURIComponent(projectName)}`,
    );
  },

  async pushToVault(
    this: void,
    request: VaultExportRequest,
    vaultId?: string,
  ): Promise<VaultPrResult> {
    return invokeOrFetch<VaultPrResult>(
      "cmd_vault_push",
      { request, vaultId },
      vaultPath(vaultId, "/push"),
      { method: "POST", body: JSON.stringify(request) },
    );
  },

  async importVaultProject(
    this: void,
    request: VaultImportRequest,
    vaultId?: string,
  ): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>(
      "cmd_vault_import",
      { request, vaultId },
      vaultPath(vaultId, "/projects"),
      { method: "POST", body: JSON.stringify({ ...request, merge: false }) },
    );
  },

  /** Adopt a vault project into the local project of the same name, keeping local repo paths. */
  async mergeVaultProject(
    this: void,
    request: VaultImportRequest,
    vaultId?: string,
  ): Promise<VaultResult> {
    return invokeOrFetch<VaultResult>(
      "cmd_vault_merge",
      { request, vaultId },
      vaultPath(vaultId, "/projects"),
      { method: "POST", body: JSON.stringify({ ...request, merge: true }) },
    );
  },

  /** Opens a PR that removes the project from the vault; the local project is untouched. */
  async deleteVaultProject(
    this: void,
    projectName: string,
    vaultId?: string,
  ): Promise<VaultPrResult> {
    return invokeOrFetch<VaultPrResult>(
      "cmd_vault_delete_project",
      { projectName, vaultId },
      vaultPath(vaultId, `/projects/${encodeURIComponent(projectName)}`),
      { method: "DELETE" },
    );
  },

  async listGitHubIssues(
    this: void,
    repo?: string,
    category?: string,
    page?: number,
    perPage?: number,
  ): Promise<GitHubIssuesPage> {
    return invoke<GitHubIssuesPage>("cmd_list_github_issues", {
      repo,
      category,
      page,
      perPage,
    });
  },

  /**
   * Forces an assigned-issue sweep. Resolves for both `Ran` and
   * `AlreadyRunning` — read `outcome` to tell them apart. Rejects when the
   * daemon is not the master, since it cannot do the work.
   */
  async checkInbox(this: void): Promise<SweepReport> {
    return invoke<SweepReport>("cmd_check_inbox");
  },

  /** Swept issues awaiting a decision. Omitting `state` returns the pending ones. */
  async listMissions(this: void): Promise<Mission[]> {
    return invoke<Mission[]>("cmd_list_missions");
  },

  async getMission(this: void, id: string): Promise<Mission> {
    return invoke<Mission>("cmd_get_mission", { id });
  },

  async createMission(
    this: void,
    request: CreateMissionRequest,
    target?: MachineTarget,
  ): Promise<Mission> {
    return invoke<Mission>("cmd_create_mission", { request, target });
  },

  /** Approve, pause, resume, cancel, or take the next step now. Answers with the mission. */
  async missionAction(
    this: void,
    id: string,
    action: MissionAction,
    body?: { reason?: string; changeRequest?: string; text?: string },
  ): Promise<Mission> {
    return invoke<Mission>("cmd_mission_action", { id, action, body });
  },

  /** The branch names `git` would produce, before it is saved (Settings → Git & Branches). */
  async previewBranchNames(this: void, git: GitSettings): Promise<BranchPreview> {
    return invoke<BranchPreview>("cmd_preview_branch_names", { git });
  },

  /** The harness per role; applies from the mission's next job. */
  async setMissionAgents(this: void, id: string, agents: MissionAgents): Promise<Mission> {
    return invoke<Mission>("cmd_set_mission_agents", { id, agents });
  },

  async setMissionBudget(this: void, id: string, budget: Partial<MissionBudget>): Promise<Mission> {
    return invoke<Mission>("cmd_set_mission_budget", { id, budget });
  },

  async listInboxProposals(this: void, state?: string): Promise<InboxProposal[]> {
    return invoke<InboxProposal[]>("cmd_list_inbox_proposals", { state });
  },

  async acceptInboxProposal(this: void, id: number): Promise<{ jobId: string }> {
    return invoke<{ jobId: string }>("cmd_accept_inbox_proposal", { id });
  },

  async dismissInboxProposal(this: void, id: number): Promise<void> {
    await invoke("cmd_dismiss_inbox_proposal", { id });
  },

  /**
   * A file on this machine as a `data:` URL an `<img>` can load, or a rejection if the daemon will not
   * serve it.
   *
   * The webview cannot load a `file://` path, so the bytes come from the daemon's guarded
   * `GET /ivy/local-file` — V1's mechanism — fetched natively because that route takes its credential
   * in the query string and the only credential the app has is the bearer secret the webview never
   * sees. Which paths are readable is the daemon's decision, not this method's: anything outside the
   * configured local-file roots, and anything that is not an allow-listed image or PDF, rejects with
   * `NOT_FOUND`. See `src-tauri/src/commands/local_file.rs`.
   *
   * A client that is not on the daemon's machine implements this shape differently — it can request the
   * route directly with a session token, which is what V1's `getAttachmentUrl` builds.
   */
  async getLocalFilePreview(this: void, path: string): Promise<string> {
    return invoke<string>("cmd_get_local_file_preview", { path });
  },

  /**
   * Copies an attached file into Tendril's own attachment directory and answers with the attachment to
   * put on the message: the user's file name, and the staged path.
   *
   * The path a file dialog or a native drop hands over is almost never inside a local-file root — a
   * screenshot on the Desktop is the ordinary case — so `getLocalFilePreview` would refuse it and the
   * message would show a paperclip chip instead of the image. Staging is what makes it previewable, and
   * is what V1's composer does by uploading every attachment before referencing it. The staged path is
   * also what the agent is told about, so the file the agent reads is the file the thumbnail shows.
   *
   * `sessionId` is the chat session the file belongs to; omitted, the daemon stages under `temp`, as
   * V1 does for a file attached before the session exists.
   */
  async uploadChatAttachment(
    this: void,
    path: string,
    sessionId?: string,
  ): Promise<ChatAttachment> {
    return invoke<ChatAttachment>("cmd_upload_chat_attachment", { path, sessionId });
  },

  /**
   * Stages a file the webview holds as bytes - one picked, dropped or pasted into a `ContentInput`,
   * which arrives as a `File` with no path - under `<TendrilHome>/Attachments/<sessionId>/`, and
   * answers with the staged attachment. `dataBase64` is the file's bytes, base64-encoded, as
   * `ContentInput`'s `OnUploadFile` event carries them.
   */
  async uploadAttachmentBytes(
    this: void,
    fileName: string,
    dataBase64: string,
    sessionId?: string,
  ): Promise<ChatAttachment> {
    return invoke<ChatAttachment>("cmd_upload_attachment_bytes", {
      fileName,
      dataBase64,
      sessionId,
    });
  },
};

/**
 * The surface every view talks to, and the seam a non-Tauri client plugs into.
 *
 * `tauriClient` reaches the daemon through Tauri IPC, which only works when the app and the daemon share
 * a machine: `invoke` is not available in a browser, and the webview holds no daemon credential of its
 * own (see #143). A web or mobile client therefore needs a different implementation of this same shape,
 * talking HTTP with a session token from `POST /api/auth/login`.
 *
 * Views import `bridge` and are indifferent to which implementation is installed. The indirection is a
 * `Proxy` rather than 100-odd delegating methods, so adding a command to `tauriClient` needs no second
 * edit here, and `TendrilClient` stays derived from the implementation instead of drifting from it.
 */
export type TendrilClient = typeof tauriClient;

let current: TendrilClient = tauriClient;

/** Installs a different client, e.g. an HTTP one for a client that is not on the daemon's machine. */
export function setTendrilClient(next: TendrilClient): void {
  current = next;
}

/** Restores the Tauri client. Tests use this to undo `setTendrilClient`. */
export function resetTendrilClient(): void {
  current = tauriClient;
}

export const bridge: TendrilClient = new Proxy({} as TendrilClient, {
  get(_target, property) {
    return (current as unknown as Record<string | symbol, unknown>)[property];
  },
  // `vi.spyOn(bridge, "listGitHubIssues")` writes the spy back onto the object, so `set` and
  // `deleteProperty` have to reach the installed client too. Without them the spy lands on the proxy's
  // own target, `get` keeps returning the original method, and the stub silently never fires.
  set(_target, property, value) {
    (current as unknown as Record<string | symbol, unknown>)[property] = value;
    return true;
  },
  deleteProperty(_target, property) {
    delete (current as unknown as Record<string | symbol, unknown>)[property];
    return true;
  },
  // `vi.spyOn` installs through `Object.defineProperty`, not a plain assignment, so this trap is what
  // actually makes spying work; without it the spy is defined on the proxy's own target and `get` keeps
  // serving the original method.
  defineProperty(_target, property, descriptor) {
    Object.defineProperty(current as object, property, descriptor);
    return true;
  },
  has(_target, property) {
    return property in (current as object);
  },
  ownKeys() {
    return Reflect.ownKeys(current as object);
  },
  getOwnPropertyDescriptor(_target, property) {
    return Reflect.getOwnPropertyDescriptor(current as object, property);
  },
});
