import { parse as parseYaml } from "yaml";
import type {
  AddedProjectRepo,
  AgentCostBreakdown,
  Annotation,
  BranchPreview,
  BridgeError,
  CreateMissionRequest,
  CreateProjectRequest,
  CreatedProject,
  CrossPlanRecommendation,
  DashboardActivity,
  DoctorCheck,
  DraftComment,
  GitHubIssuesPage,
  GitSettings,
  InboxProposal,
  IssueMetadata,
  Job,
  JobDetail,
  Machine,
  Mission,
  MissionAction,
  MissionAgents,
  MissionBudget,
  ModelCatalogStatus,
  OnboardingStatus,
  PlanArtifactContent,
  PlanCommitDetail,
  PlanDetail,
  PlanGitData,
  PlanQuery,
  PlanSummary,
  PrStatus,
  PrSyncReport,
  ProjectSummary,
  ProvisionReport,
  RecentMergedPr,
  RecentPlanCost,
  RecommendationItem,
  RecommendationState,
  RepoStatus,
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
  VerificationReport,
  VerificationStatus,
  VersionInfo,
} from "../types/api";
import type { ChatAttachment } from "../types/chat";
import { setTendrilClient, tauriClient, type TendrilClient } from "./bridge";
import { decodeBase64 } from "./events";

/**
 * The `TendrilClient` for a browser: the same shape as `tauriClient`, over plain `fetch`.
 *
 * Every Tauri command is a thin wrapper over one daemon route, so each method here calls the route
 * its command's `TendrilClient` method in `src-tauri/src/service/client/` calls, and reshapes the
 * answer the way that method does - `plan_mapping.rs` for plans, the job projections in `jobs.rs`,
 * and so on - so views cannot tell the transports apart.
 *
 * Requests are same-origin and carry no credential. The page is served by an authenticating proxy in
 * front of the daemon (coding-env's `tendril-web.mjs`): the browser holds that proxy's session
 * cookie, and the proxy adds the daemon's bearer secret, so the secret never reaches this code - the
 * property the desktop app gets by keeping it native (#143).
 *
 * Methods the `tauriClient` already implements over HTTP outside Tauri (`invokeOrFetch`, the review
 * action stream) are inherited unchanged. What only the native side can do - manage the local
 * service, read git or plan files off the app's own disk, run `gh` or the CLI - rejects with
 * `UNSUPPORTED_IN_BROWSER`, or answers the empty value the view already renders for "nothing here".
 */

const UNASSIGNED_SESSION = "temp";
const MAX_ATTACHMENT_BYTES = 16 * 1024 * 1024;
const MAX_PREVIEW_BYTES = 16 * 1024 * 1024;
const UI_STATE_PREFIX = "tendril-ui-state:";

type Json = Record<string, unknown>;

function bridgeError(code: string, message: string, details?: string): BridgeError {
  return { code, message, details: details ?? null };
}

function unsupported(what: string): BridgeError {
  return bridgeError("UNSUPPORTED_IN_BROWSER", `${what} is only available in the desktop app`);
}

function segment(value: string): string {
  return encodeURIComponent(value);
}

function withQuery(
  path: string,
  params: Record<string, string | number | boolean | undefined | null>,
) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null) query.set(key, String(value));
  }
  const qs = query.toString();
  return qs ? `${path}?${qs}` : path;
}

interface RequestOptions {
  method?: string;
  body?: unknown;
  /** The `code` of a non-2xx rejection; `409` is always `CONFLICT`, as `expect_success` maps it. */
  code: string;
  /** Completes "Could not …" when the daemon's answer carries no `error` of its own. */
  action: string;
  /** When set, a `404` rejects as `NOT_FOUND` with this message, as `get_plan` does. */
  notFound?: string;
}

async function send(path: string, options: RequestOptions): Promise<Response> {
  const init: RequestInit = { method: options.method ?? "GET", headers: {} };
  if (options.body !== undefined) {
    init.body = JSON.stringify(options.body);
    (init.headers as Record<string, string>)["Content-Type"] = "application/json";
  }

  let response: Response;
  try {
    response = await fetch(path, init);
  } catch (err) {
    throw bridgeError("DISCONNECTED", "Tendril is not reachable", String(err));
  }
  if (response.ok) return response;

  const text = await response.text().catch(() => "");
  let serviceMessage: string | undefined;
  try {
    const parsed = JSON.parse(text) as { error?: unknown };
    if (typeof parsed.error === "string") serviceMessage = parsed.error;
  } catch {
    // Not JSON; the status says enough.
  }
  const message = serviceMessage ?? `Could not ${options.action} (${response.status})`;
  if (response.status === 409) throw bridgeError("CONFLICT", message);
  if (response.status === 404 && options.notFound) throw bridgeError("NOT_FOUND", options.notFound);
  if (response.status === 401) throw bridgeError("UNAUTHENTICATED", message, text);
  throw bridgeError(options.code, message, text);
}

async function getJson<T>(path: string, options: Omit<RequestOptions, "method">): Promise<T> {
  return (await send(path, options)).json() as Promise<T>;
}

async function sendJson<T>(path: string, options: RequestOptions): Promise<T> {
  return (await send(path, options)).json() as Promise<T>;
}

async function sendVoid(path: string, options: RequestOptions): Promise<void> {
  await send(path, options);
}

/* --- Value helpers, the `serde_json::Value` accessors the Rust mappers use ---------------------- */

function asString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function str(source: unknown, key: string): string | undefined {
  return asString((source as Json | undefined)?.[key]);
}

/** `string_field`: a non-empty string, or nothing. */
function nonEmpty(source: unknown, key: string): string | undefined {
  const value = str(source, key);
  return value ? value : undefined;
}

/** A trimmed non-empty string, or nothing - `detail_text` in `jobs.rs`. */
function trimmed(source: unknown, key: string): string | undefined {
  const value = str(source, key)?.trim();
  return value ? value : undefined;
}

function num(source: unknown, key: string): number | undefined {
  const value = (source as Json | undefined)?.[key];
  return typeof value === "number" ? value : undefined;
}

function int(source: unknown, key: string): number | undefined {
  const value = num(source, key);
  return value !== undefined && Number.isInteger(value) ? value : undefined;
}

function bool(source: unknown, key: string): boolean | undefined {
  const value = (source as Json | undefined)?.[key];
  return typeof value === "boolean" ? value : undefined;
}

function strings(source: unknown, key: string): string[] {
  const value = (source as Json | undefined)?.[key];
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

/** The first of several keys that is present - `val.get(a).or_else(|| val.get(b))`. */
function first(source: unknown, ...keys: string[]): unknown {
  for (const key of keys) {
    const value = (source as Json | undefined)?.[key];
    if (value !== undefined && value !== null) return value;
  }
  return undefined;
}

/* --- Plans: a port of `service/plan_mapping.rs` ------------------------------------------------- */

interface PlanYamlExtras {
  priority?: number;
  executionProfile?: string;
  recommendations?: RecommendationItem[];
  verifications?: { name: string; status: string }[];
  repos?: string[];
  allocatedPorts?: Record<string, number>;
}

function parsePlanYaml(raw: unknown): PlanYamlExtras {
  if (typeof raw !== "string") return {};
  try {
    const parsed = parseYaml(raw) as Json | null;
    if (!parsed || typeof parsed !== "object") return {};
    const extras: PlanYamlExtras = {};
    if (typeof parsed.priority === "number") extras.priority = parsed.priority;
    if (typeof parsed.executionProfile === "string")
      extras.executionProfile = parsed.executionProfile;
    if (Array.isArray(parsed.recommendations)) {
      extras.recommendations = parsed.recommendations as RecommendationItem[];
    }
    if (Array.isArray(parsed.verifications)) {
      extras.verifications = verificationList(parsed.verifications);
    }
    if (Array.isArray(parsed.repos)) {
      extras.repos = parsed.repos.filter((repo): repo is string => typeof repo === "string");
    }
    if (parsed.allocatedPorts && typeof parsed.allocatedPorts === "object") {
      extras.allocatedPorts = parsed.allocatedPorts as Record<string, number>;
    }
    return extras;
  } catch {
    return {};
  }
}

function verificationList(items: unknown[]): { name: string; status: string }[] {
  return items.flatMap((item) => {
    const name = str(item, "name");
    return name === undefined ? [] : [{ name, status: str(item, "status") ?? "Pending" }];
  });
}

/** Plan ids are zero-padded 5-digit strings everywhere, but `PlanMetadata.id` is a number. */
function planId(metadata: unknown, fallback: string): string {
  const id = (metadata as Json | undefined)?.id;
  if (typeof id === "number" && Number.isInteger(id)) return String(id).padStart(5, "0");
  return typeof id === "string" && id ? id : fallback;
}

function resolveVerifications(metadata: unknown, extras: PlanYamlExtras) {
  const raw = (metadata as Json | undefined)?.verifications;
  const fromMetadata = Array.isArray(raw) ? verificationList(raw) : [];
  return fromMetadata.length > 0 ? fromMetadata : (extras.verifications ?? []);
}

function resolveRecommendations(metadata: unknown, extras: PlanYamlExtras): RecommendationItem[] {
  const raw = (metadata as Json | undefined)?.recommendations;
  if (Array.isArray(raw) && raw.length > 0) return raw as RecommendationItem[];
  return extras.recommendations ?? [];
}

function resolveRepos(metadata: unknown, extras: PlanYamlExtras): string[] {
  const fromMetadata = strings(metadata, "repos");
  return fromMetadata.length > 0 ? fromMetadata : (extras.repos ?? []);
}

function mapPlanSummary(value: unknown, fallbackId: string): PlanSummary {
  const metadata = (value as Json).metadata ?? value;
  const extras = parsePlanYaml((value as Json).yaml_raw);
  return {
    id: planId(metadata, fallbackId),
    title: nonEmpty(metadata, "title") ?? "",
    state: (nonEmpty(metadata, "state") ?? "Draft") as PlanSummary["state"],
    project: nonEmpty(metadata, "project") ?? "",
    level: nonEmpty(metadata, "level") ?? "Feature",
    priority: extras.priority,
    created: nonEmpty(metadata, "created"),
    updated: nonEmpty(metadata, "updated"),
    verifications: resolveVerifications(metadata, extras) as PlanSummary["verifications"],
    allocatedPorts: extras.allocatedPorts,
  };
}

function mapPlanDetail(value: unknown, fallbackId: string): PlanDetail {
  const metadata = (value as Json).metadata ?? value;
  const extras = parsePlanYaml((value as Json).yaml_raw);
  return {
    id: planId(metadata, fallbackId),
    title: nonEmpty(metadata, "title") ?? "",
    state: (nonEmpty(metadata, "state") ?? "Draft") as PlanDetail["state"],
    project: nonEmpty(metadata, "project") ?? "",
    level: nonEmpty(metadata, "level") ?? "Feature",
    priority: extras.priority,
    executionProfile: extras.executionProfile,
    initialPrompt: nonEmpty(metadata, "initial_prompt"),
    sourceUrl: nonEmpty(metadata, "source_url"),
    created: nonEmpty(metadata, "created"),
    updated: nonEmpty(metadata, "updated"),
    repos: resolveRepos(metadata, extras),
    verifications: resolveVerifications(metadata, extras) as PlanDetail["verifications"],
    dependsOn: strings(metadata, "depends_on"),
    relatedPlans: strings(metadata, "related_plans"),
    commits: strings(metadata, "commits"),
    prs: strings(metadata, "prs"),
    latestRevisionContent: nonEmpty(value, "latest_revision_content"),
    folderPath: nonEmpty(value, "folder_path"),
    revisionCount: int(value, "revision_count") ?? 0,
    recommendations: resolveRecommendations(metadata, extras),
    allocatedPorts: extras.allocatedPorts,
  };
}

/* --- Jobs: a port of the projections in `service/client/jobs.rs` -------------------------------- */

/** `plan_id_from_folder`: the leading digits of a plan folder name, zero-padded to five. */
function planIdFromFolder(planFile: string): string | undefined {
  const name = (planFile.split(/[/\\]/).pop() ?? planFile).trim();
  const digits = /^\d+/.exec(name)?.[0];
  if (!digits) return undefined;
  const parsed = Number.parseInt(digits, 10);
  return Number.isSafeInteger(parsed) ? String(parsed).padStart(5, "0") : undefined;
}

/** `JobArgs::prompt_text`: the one free-text field of each job type, when it has any text. */
function promptFromTypedArgs(typedArgs: unknown): string | undefined {
  if (!typedArgs || typeof typedArgs !== "object") return undefined;
  const fields: Record<string, string> = {
    CreatePlan: "description",
    RetryPlan: "changeRequest",
    UpdatePlan: "instructions",
    ExecutePlan: "note",
    CreatePr: "comment",
    CreateIssue: "comment",
    SyncRepo: "repoPath",
    AddProject: "projectName",
    SetupProject: "folderPath",
  };
  const field = fields[str(typedArgs, "type") ?? ""];
  const text = field ? str(typedArgs, field) : undefined;
  return text && text.trim() ? text : undefined;
}

function jobPlanId(source: unknown): string | undefined {
  const reported = first(source, "reportedPlanId", "planId");
  if (typeof reported === "string" && reported.trim()) return reported;
  const planFile = str(source, "planFile");
  return planFile ? planIdFromFolder(planFile) : undefined;
}

function jobCommon(source: unknown): Job {
  return {
    id: str(source, "id") ?? "",
    type: str(source, "type") ?? "Unknown",
    planId: jobPlanId(source),
    planTitle: asString(first(source, "reportedPlanTitle", "planTitle")),
    project: str(source, "project") ?? "",
    status: (str(source, "status") ?? "Pending") as Job["status"],
    statusMessage: str(source, "statusMessage"),
    startedAt: str(source, "startedAt"),
    completedAt: str(source, "completedAt"),
    cost: num(source, "cost"),
    tokens: int(source, "tokens"),
    costSource: str(source, "costSource"),
    durationSeconds: int(source, "durationSeconds"),
    inputTokens: int(source, "inputTokens"),
    outputTokens: int(source, "outputTokens"),
    cacheReadTokens: int(source, "cacheReadTokens"),
    cacheWriteTokens: int(source, "cacheWriteTokens"),
    reasoningTokens: int(source, "reasoningTokens"),
    model: str(source, "model"),
    processId: int(source, "processId"),
    detached: bool(source, "detached"),
  } as Job;
}

function mapJob(value: unknown): Job {
  return {
    ...jobCommon(value),
    prompt: str(value, "prompt") ?? promptFromTypedArgs((value as Json).typedArgs),
    lastOutputAt: str(value, "lastOutputAt"),
    chatSessionId: str(value, "chatSessionId"),
  };
}

function mapJobDetail(value: unknown, jobId: string): JobDetail {
  const details = (value as Json).details ?? value;
  const job = jobCommon(details);
  return {
    ...job,
    id: job.id || jobId,
    prompt:
      str(value, "prompt") ??
      str(details, "prompt") ??
      promptFromTypedArgs(first(details, "typedArgs") ?? (value as Json).typedArgs),
    args: str(details, "args"),
    workingDirectory: str(details, "workingDirectory"),
    reportedFailureReason: str(details, "reportedFailureReason"),
    permissionDenials: strings(details, "permissionDenials"),
    provider: trimmed(details, "provider"),
    cliCommand: trimmed(details, "cliCommand"),
    executionProfile: trimmed(details, "executionProfile"),
    planFolder: trimmed(details, "planFile"),
    lastOutputAt: trimmed(details, "lastOutputAt"),
    // The `job*Path` artifact locations are resolved on the app's own disk; a browser has none.
  };
}

function startJobResponse(value: unknown): StartJobResponse {
  return { jobId: str(value, "jobId") ?? "", status: str(value, "status") ?? "Started" };
}

/* --- Projects: `list_projects` in `service/client/projects.rs` ---------------------------------- */

function mapProject(value: unknown): ProjectSummary {
  const nameOrPath = (items: unknown, key: string): string[] =>
    Array.isArray(items)
      ? items.flatMap((item) => {
          if (typeof item === "string") return [item];
          const nested = str(item, key);
          return nested === undefined ? [] : [nested];
        })
      : [];
  const reviewActions = first(value, "reviewActions", "review_actions");
  return {
    name: str(value, "name") ?? "",
    color: str(value, "color")?.trim() || undefined,
    repos: nameOrPath((value as Json).repos, "path"),
    verifications: nameOrPath((value as Json).verifications, "name"),
    reviewActions: Array.isArray(reviewActions) ? (reviewActions as ReviewActionConfig[]) : [],
  };
}

/* --- Browser-only helpers ----------------------------------------------------------------------- */

function readUiState(key: string): string | null {
  try {
    return window.localStorage.getItem(UI_STATE_PREFIX + key);
  } catch {
    return null;
  }
}

function writeUiState(key: string, value: string): void {
  try {
    window.localStorage.setItem(UI_STATE_PREFIX + key, value);
  } catch {
    // Private windows and full quotas refuse; losing a remembered panel width is not an error.
  }
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    // `readAsDataURL` always yields a string.
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

async function health(): Promise<Json> {
  return getJson<Json>("/api/health", { code: "HEALTH_FAILED", action: "reach Tendril" });
}

export const httpClient: TendrilClient = {
  ...tauriClient,

  /* --- Service ---------------------------------------------------------------------------------
     The daemon is whatever the proxy fronts. There is no local process to supervise, and no daemon
     origin to report: leaving `host`/`port` unset keeps `ProxyOriginProvider` on same-origin, which
     is where the WebViewer and wireframe routes are served from here. */

  async checkServiceHealth(this: void): Promise<ServiceHealth> {
    const body = await health();
    return {
      status: "Healthy",
      isHealthy: true,
      apiVersion: int(body, "apiVersion"),
      capabilities: strings(body, "capabilities"),
    };
  },

  async getServiceInfo(this: void): Promise<ServiceInfo> {
    const body = await health();
    const onboarding = await getJson<Json>("/api/onboarding", {
      code: "GET_ONBOARDING_STATUS_FAILED",
      action: "read onboarding status",
    }).catch(() => ({}) as Json);
    return {
      state: "Connected",
      tendrilHome: str(onboarding, "tendrilHome") ?? "",
      version: str(body, "version"),
      apiVersion: int(body, "apiVersion"),
      pid: int(body, "pid"),
      capabilities: strings(body, "capabilities"),
      message: "Connected through the Tendril web UI",
      ownership: "Remote",
      statusBadge: "Connected (Web)",
    };
  },

  async getServiceLogs(this: void): Promise<string[]> {
    return [];
  },

  async restartService(this: void): Promise<ServiceInfo> {
    throw unsupported("Restarting the service");
  },

  async repairService(this: void): Promise<string> {
    throw unsupported("Repairing the service");
  },

  async switchServiceMode(this: void): Promise<ServiceInfo> {
    throw unsupported("Switching the service mode");
  },

  async installService(this: void): Promise<ProvisionReport> {
    throw unsupported("Installing the service");
  },

  async uninstallServiceAutostart(this: void): Promise<string> {
    throw unsupported("Changing the service autostart");
  },

  /* --- Plans ----------------------------------------------------------------------------------- */

  async listPlans(this: void, query?: PlanQuery): Promise<PlanSummary[]> {
    const raw = await getJson<unknown[]>(
      withQuery("/api/plans", { status: query?.status, project: query?.project, q: query?.q }),
      { code: "LIST_PLANS_FAILED", action: "list plans" },
    );
    return raw.map((value) => mapPlanSummary(value, ""));
  },

  async getPlan(this: void, id: string): Promise<PlanDetail> {
    const raw = await getJson<unknown>(`/api/plans/${segment(id)}`, {
      code: "GET_PLAN_FAILED",
      action: `get plan '${id}'`,
      notFound: `Plan '${id}' not found`,
    });
    return mapPlanDetail(raw, id);
  },

  async updatePlanField(
    this: void,
    id: string,
    field: string,
    value: string,
    allowFailed?: boolean,
  ): Promise<void> {
    await sendVoid(`/api/plans/${segment(id)}`, {
      method: "PUT",
      body: { field, value, allowFailedVerifications: allowFailed ?? false },
      code: "UPDATE_FIELD_FAILED",
      action: "update the plan field",
    });
  },

  async deletePlan(this: void, id: string): Promise<void> {
    await sendVoid(`/api/plans/${segment(id)}`, {
      method: "DELETE",
      code: "DELETE_PLAN_FAILED",
      action: `delete plan '${id}'`,
    });
  },

  async resetPlan(this: void, id: string): Promise<void> {
    await sendVoid(`/api/plans/${segment(id)}/reset`, {
      method: "POST",
      code: "RESET_PLAN_FAILED",
      action: `reset plan '${id}'`,
    });
  },

  async getRepoStatus(this: void, id: string): Promise<RepoStatus[]> {
    const body = await getJson<{ repos?: RepoStatus[] }>(`/api/plans/${segment(id)}/repo-status`, {
      code: "REPO_STATUS_FAILED",
      action: `read repo status for plan '${id}'`,
    });
    return body.repos ?? [];
  },

  async getProjectRepoStatus(this: void, projectName: string): Promise<RepoStatus[]> {
    const body = await getJson<{ repos?: RepoStatus[] }>(
      `/api/projects/${segment(projectName)}/repo-status`,
      { code: "REPO_STATUS_FAILED", action: `read repo status for project '${projectName}'` },
    );
    return body.repos ?? [];
  },

  async getProjectIssueMetadata(this: void, projectName: string): Promise<IssueMetadata> {
    return getJson<IssueMetadata>(`/api/projects/${segment(projectName)}/issues/metadata`, {
      code: "ISSUE_METADATA_FAILED",
      action: `read issue metadata for project '${projectName}'`,
    });
  },

  async getPlanGit(this: void, id: string): Promise<PlanGitData> {
    return getJson<PlanGitData>(`/api/plans/${segment(id)}/git`, {
      code: "PLAN_GIT_FAILED",
      action: `read git state for plan '${id}'`,
    });
  },

  /** Read from the repos on the app's own disk (`commands::plan_files`); "no longer held" here. */
  async getPlanCommit(this: void): Promise<PlanCommitDetail | null> {
    return null;
  },

  async getPlanFileContent(this: void): Promise<PlanArtifactContent> {
    throw unsupported("Opening a linked local file");
  },

  async getRevision(this: void, id: string, number?: number): Promise<string> {
    const response = await send(withQuery(`/api/plans/${segment(id)}/revisions`, { number }), {
      code: "GET_REVISION_FAILED",
      action: `get a revision of plan '${id}'`,
    });
    return response.text();
  },

  async writeRevision(this: void, id: string, content: string): Promise<RevisionResult> {
    const body = await sendJson<Json>(`/api/plans/${segment(id)}/revisions`, {
      method: "POST",
      body: { content },
      code: "WRITE_REVISION_FAILED",
      action: "write the revision",
    });
    return {
      revision: int(body, "revision") ?? 1,
      message: str(body, "message") ?? "Revision written",
    };
  },

  async updateLatestRevision(this: void, id: string, content: string): Promise<RevisionResult> {
    const body = await sendJson<Json>(`/api/plans/${segment(id)}/revisions/latest`, {
      method: "PUT",
      body: { content },
      code: "UPDATE_LATEST_REVISION_FAILED",
      action: `update the latest revision of plan '${id}'`,
    });
    return {
      revision: int(body, "revision") ?? 0,
      message: str(body, "message") ?? "Revision updated",
    };
  },

  async listDiffComments(this: void, planId: string): Promise<DraftComment[]> {
    return getJson<DraftComment[]>(`/api/plans/${segment(planId)}/diff-comments`, {
      code: "LIST_DIFF_COMMENTS_FAILED",
      action: "list diff comments",
    });
  },

  async upsertDiffComment(
    this: void,
    planId: string,
    comment: DraftComment,
  ): Promise<DraftComment[]> {
    return sendJson<DraftComment[]>(`/api/plans/${segment(planId)}/diff-comments`, {
      method: "POST",
      body: comment,
      code: "UPSERT_DIFF_COMMENT_FAILED",
      action: "save the diff comment",
    });
  },

  async deleteDiffComment(
    this: void,
    planId: string,
    filePath: string,
    changeKey: string,
  ): Promise<DraftComment[]> {
    return sendJson<DraftComment[]>(
      withQuery(`/api/plans/${segment(planId)}/diff-comments`, { filePath, changeKey }),
      { method: "DELETE", code: "DELETE_DIFF_COMMENT_FAILED", action: "delete the diff comment" },
    );
  },

  async clearDiffComments(this: void, planId: string): Promise<void> {
    await sendVoid(`/api/plans/${segment(planId)}/diff-comments`, {
      method: "DELETE",
      code: "CLEAR_DIFF_COMMENTS_FAILED",
      action: "clear the diff comments",
    });
  },

  async listAnnotations(this: void, planId: string): Promise<Annotation[]> {
    return getJson<Annotation[]>(`/api/plans/${segment(planId)}/annotations`, {
      code: "LIST_ANNOTATIONS_FAILED",
      action: "list annotations",
    });
  },

  async upsertAnnotation(
    this: void,
    planId: string,
    annotation: Annotation,
  ): Promise<Annotation[]> {
    return sendJson<Annotation[]>(`/api/plans/${segment(planId)}/annotations`, {
      method: "POST",
      body: annotation,
      code: "UPSERT_ANNOTATION_FAILED",
      action: "save the annotation",
    });
  },

  async deleteAnnotation(this: void, planId: string, annotationId: string): Promise<Annotation[]> {
    return sendJson<Annotation[]>(
      withQuery(`/api/plans/${segment(planId)}/annotations`, { id: annotationId }),
      { method: "DELETE", code: "DELETE_ANNOTATION_FAILED", action: "delete the annotation" },
    );
  },

  async clearAnnotations(this: void, planId: string): Promise<void> {
    await sendVoid(`/api/plans/${segment(planId)}/annotations`, {
      method: "DELETE",
      code: "CLEAR_ANNOTATIONS_FAILED",
      action: "clear the annotations",
    });
  },

  /** Reports are read from the plan folder on the app's own disk (`verification_reports.rs`). */
  async getVerificationReport(
    this: void,
    _planId: string,
    name: string,
  ): Promise<VerificationReport> {
    throw bridgeError("NOT_FOUND", `The '${name}' report can only be opened in the desktop app`);
  },

  async listVerificationReports(this: void): Promise<VerificationReport[]> {
    return [];
  },

  async setVerificationStatus(
    this: void,
    planId: string,
    name: string,
    status: VerificationStatus,
  ): Promise<void> {
    await sendVoid(`/api/plans/${segment(planId)}/verifications/${segment(name)}`, {
      method: "PUT",
      body: { status },
      code: "VERIFICATION_UPDATE_FAILED",
      action: `set verification '${name}' to ${status}`,
    });
  },

  async listRecommendations(this: void, planId: string): Promise<RecommendationItem[]> {
    return (await httpClient.getPlan(planId)).recommendations;
  },

  /** No daemon route answers this directly; the desktop app gathers it plan by plan, and so does this. */
  async listCrossPlanRecommendations(
    this: void,
    project?: string,
    state?: string,
  ): Promise<CrossPlanRecommendation[]> {
    const plans = await httpClient.listPlans();
    const relevant = plans.filter((plan) => plan.state === "Completed" || !state);
    const results: CrossPlanRecommendation[] = [];
    await Promise.all(
      relevant.map(async (plan) => {
        try {
          for (const r of await httpClient.listRecommendations(plan.id)) {
            if (state && (r.state || "Pending") !== state) continue;
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
          // One unreadable plan does not hide the others' recommendations.
        }
      }),
    );
    return results;
  },

  async setRecommendationState(
    this: void,
    planId: string,
    title: string,
    state: RecommendationState,
    declineReason?: string,
    notes?: string,
  ): Promise<void> {
    await sendVoid(`/api/plans/${segment(planId)}/recommendations/${segment(title)}`, {
      method: "PUT",
      body: { state, declineReason: declineReason ?? null, notes: notes ?? null },
      code: "RECOMMENDATION_UPDATE_FAILED",
      action: `set recommendation '${title}' to ${state}`,
    });
  },

  /* --- Jobs ------------------------------------------------------------------------------------ */

  async listJobs(this: void, status?: string, limit?: number): Promise<Job[]> {
    const raw = await getJson<unknown[]>(withQuery("/api/jobs", { status, limit }), {
      code: "LIST_JOBS_FAILED",
      action: "list jobs",
    });
    return raw.map(mapJob);
  },

  async getJob(this: void, id: string): Promise<JobDetail> {
    const raw = await getJson<unknown>(`/api/jobs/${segment(id)}`, {
      code: "GET_JOB_FAILED",
      action: `get job '${id}'`,
    });
    return mapJobDetail(raw, id);
  },

  /** `target` picks between two daemons only when the desktop app is connected to a remote one. */
  async startJob(this: void, args: StartJobArgs): Promise<StartJobResponse> {
    const body = await sendJson<unknown>("/api/jobs", {
      method: "POST",
      body: args,
      code: "START_JOB_FAILED",
      action: "start the job",
    });
    return startJobResponse(body);
  },

  async listMachines(this: void): Promise<Machine[]> {
    return [];
  },

  async cancelJob(this: void, id: string, message?: string): Promise<void> {
    await sendVoid(`/api/jobs/${segment(id)}/cancel`, {
      method: "POST",
      body: { message: message ?? null },
      code: "CANCEL_JOB_FAILED",
      action: "cancel the job",
    });
  },

  async deleteJob(this: void, id: string): Promise<void> {
    await sendVoid(`/api/jobs/${segment(id)}`, {
      method: "DELETE",
      code: "DELETE_JOB_FAILED",
      action: `delete job '${id}'`,
    });
  },

  async forceStartJob(this: void, id: string): Promise<void> {
    await sendVoid(`/api/jobs/${segment(id)}/force-start`, {
      method: "POST",
      code: "FORCE_START_JOB_FAILED",
      action: `force-start job '${id}'`,
    });
  },

  async rerunJob(this: void, id: string, feedback?: string): Promise<StartJobResponse> {
    const body = await sendJson<unknown>(`/api/jobs/${segment(id)}/rerun`, {
      method: "POST",
      body: { feedback: feedback?.trim() || null },
      code: "RERUN_JOB_FAILED",
      action: `rerun job '${id}'`,
    });
    return startJobResponse(body);
  },

  async reportJobBug(this: void): Promise<string> {
    throw unsupported("Reporting a bug");
  },

  async clearJobs(this: void, status: string): Promise<number> {
    const body = await sendJson<Json>("/api/jobs/clear", {
      method: "POST",
      body: { status },
      code: "CLEAR_JOBS_FAILED",
      action: `clear '${status}' jobs`,
    });
    return int(body, "cleared") ?? 0;
  },

  /* --- Projects -------------------------------------------------------------------------------- */

  async listProjects(this: void): Promise<ProjectSummary[]> {
    const raw = await getJson<unknown[]>("/api/projects", {
      code: "LIST_PROJECTS_FAILED",
      action: "list projects",
    });
    return raw.map(mapProject);
  },

  async listPullRequests(this: void): Promise<PrStatus[]> {
    return getJson<PrStatus[]>("/api/pull-requests", {
      code: "LIST_PULL_REQUESTS_FAILED",
      action: "list pull requests",
    });
  },

  async syncPullRequests(this: void): Promise<PrSyncReport> {
    try {
      return await sendJson<PrSyncReport>("/api/pull-requests/sync", {
        method: "POST",
        code: "SYNC_PULL_REQUESTS_FAILED",
        action: "sync pull requests",
      });
    } catch (err) {
      if ((err as BridgeError).code === "CONFLICT") {
        throw bridgeError("PR_SYNC_IN_PROGRESS", "A pull request sync is already running");
      }
      throw err;
    }
  },

  async getReviewActionConditions(
    this: void,
    projectName: string,
    planId: string,
  ): Promise<ReviewActionConditionResult[]> {
    return getJson<ReviewActionConditionResult[]>(
      withQuery(`/api/projects/${segment(projectName)}/review-actions`, { planId }),
      { code: "REVIEW_ACTION_CONDITIONS_FAILED", action: "evaluate review action conditions" },
    );
  },

  async createProject(this: void, request: CreateProjectRequest): Promise<CreatedProject> {
    return sendJson<CreatedProject>("/api/projects", {
      method: "POST",
      body: request,
      code: "CREATE_PROJECT_FAILED",
      action: "create the project",
    });
  },

  async addProjectRepo(this: void, projectName: string, path: string): Promise<AddedProjectRepo> {
    return sendJson<AddedProjectRepo>(`/api/projects/${segment(projectName)}/repos`, {
      method: "POST",
      body: { path },
      code: "ADD_PROJECT_REPO_FAILED",
      action: "add the repository",
    });
  },

  async renameProject(this: void, name: string, newName: string): Promise<string> {
    const renamed = await sendJson<{ name?: string }>(`/api/projects/${segment(name)}`, {
      method: "PUT",
      body: { newName },
      code: "RENAME_PROJECT_FAILED",
      action: "rename the project",
    }).catch((err: BridgeError) => {
      // The dialogs read every failure of this route as `RENAME_PROJECT_FAILED`, 409 included.
      throw { ...err, code: "RENAME_PROJECT_FAILED" };
    });
    return renamed?.name ?? newName;
  },

  async removeProject(this: void, name: string): Promise<void> {
    await sendVoid(`/api/projects/${segment(name)}`, {
      method: "DELETE",
      code: "REMOVE_PROJECT_FAILED",
      action: "remove the project",
    });
  },

  async deleteProjectData(this: void, name: string): Promise<void> {
    await sendVoid(`/api/projects/${segment(name)}/data`, {
      method: "DELETE",
      code: "DELETE_PROJECT_DATA_FAILED",
      action: "delete the project data",
    });
  },

  /* --- Config, onboarding, diagnostics --------------------------------------------------------- */

  async getConfig(this: void): Promise<TendrilConfig> {
    const raw = await getJson<Json>("/api/config", {
      code: "GET_CONFIG_FAILED",
      action: "get config",
    });
    return {
      codingAgent: str(raw, "codingAgent"),
      jobTimeout: int(raw, "jobTimeout"),
      maxConcurrentJobs: int(raw, "maxConcurrentJobs"),
      planTemplate: str(raw, "planTemplate"),
      theme: str(raw, "theme"),
      desktopNotifications: bool(raw, "desktopNotifications"),
      raw,
    };
  },

  async putConfig(this: void, key: string, value: unknown): Promise<void> {
    await sendVoid("/api/config", {
      method: "PUT",
      body: { [key]: value },
      code: "PUT_CONFIG_FAILED",
      action: `put config key '${key}'`,
    });
  },

  async getOnboardingStatus(this: void): Promise<OnboardingStatus> {
    return getJson<OnboardingStatus>("/api/onboarding", {
      code: "GET_ONBOARDING_STATUS_FAILED",
      action: "get onboarding status",
    });
  },

  async completeOnboarding(this: void): Promise<void> {
    await sendVoid("/api/onboarding/complete", {
      method: "POST",
      body: {},
      code: "COMPLETE_ONBOARDING_FAILED",
      action: "complete onboarding",
    });
  },

  async dismissOnboarding(this: void): Promise<void> {
    await sendVoid("/api/onboarding/dismiss", {
      method: "POST",
      body: {},
      code: "DISMISS_ONBOARDING_FAILED",
      action: "dismiss onboarding",
    });
  },

  async subscribeNewsletter(this: void, email: string): Promise<SubscribeOutcome> {
    return sendJson<SubscribeOutcome>("/api/newsletter/subscribe", {
      method: "POST",
      body: { email },
      code: "SUBSCRIBE_NEWSLETTER_FAILED",
      action: "subscribe to the newsletter",
    });
  },

  async runDoctor(this: void): Promise<DoctorCheck[]> {
    return getJson<DoctorCheck[]>("/api/doctor", {
      code: "RUN_DOCTOR_FAILED",
      action: "run health checks",
    });
  },

  async getModelsStatus(this: void): Promise<ModelCatalogStatus> {
    return getJson<ModelCatalogStatus>("/api/models/status", {
      code: "GET_MODELS_STATUS_FAILED",
      action: "get the model catalog status",
    });
  },

  async refreshModels(this: void): Promise<ModelCatalogStatus> {
    return sendJson<ModelCatalogStatus>("/api/models/refresh", {
      method: "POST",
      code: "REFRESH_MODELS_FAILED",
      action: "refresh the model catalog",
    });
  },

  async getVersionInfo(this: void): Promise<VersionInfo> {
    return getJson<VersionInfo>("/api/version", {
      code: "GET_VERSION_INFO_FAILED",
      action: "get version info",
    });
  },

  async checkVersionNow(this: void): Promise<VersionInfo> {
    return sendJson<VersionInfo>("/api/version/check", {
      method: "POST",
      code: "CHECK_VERSION_NOW_FAILED",
      action: "check the version",
    });
  },

  /** `ui_state.json` is the desktop app's own file; a browser keeps the same keys in its storage. */
  async saveUiState(this: void, key: string, value: string): Promise<void> {
    writeUiState(key, value);
  },

  async loadUiState(this: void, key: string): Promise<string | null> {
    return readUiState(key);
  },

  /* --- Dashboard ------------------------------------------------------------------------------- */

  async getDashboardActivity(this: void, months?: number): Promise<DashboardActivity> {
    return getJson<DashboardActivity>(withQuery("/api/dashboard/activity", { months }), {
      code: "GET_DASHBOARD_ACTIVITY_FAILED",
      action: "get dashboard activity",
    });
  },

  async getShippedFeatures(this: void, days?: number): Promise<ShippedFeatureDay[]> {
    return getJson<ShippedFeatureDay[]>(withQuery("/api/dashboard/shipped-features", { days }), {
      code: "GET_SHIPPED_FEATURES_FAILED",
      action: "get shipped features",
    });
  },

  async getRecentMergedPrs(this: void, limit?: number): Promise<RecentMergedPr[]> {
    return getJson<RecentMergedPr[]>(withQuery("/api/dashboard/merged-prs", { limit }), {
      code: "GET_MERGED_PRS_FAILED",
      action: "get merged PRs",
    });
  },

  async getRecentPlanCosts(this: void, days?: number): Promise<RecentPlanCost[]> {
    return getJson<RecentPlanCost[]>(withQuery("/api/dashboard/plan-costs", { days }), {
      code: "GET_PLAN_COSTS_FAILED",
      action: "get plan costs",
    });
  },

  async getAgentCostBreakdown(this: void, days?: number): Promise<AgentCostBreakdown[]> {
    return getJson<AgentCostBreakdown[]>(withQuery("/api/dashboard/agent-costs", { days }), {
      code: "GET_AGENT_COSTS_FAILED",
      action: "get the agent cost breakdown",
    });
  },

  /* --- GitHub, inbox, missions ----------------------------------------------------------------- */

  /** Runs `gh` on the app's own machine (`commands/github.rs`). */
  async listGitHubIssues(this: void): Promise<GitHubIssuesPage> {
    throw unsupported("Browsing GitHub issues");
  },

  async checkInbox(this: void): Promise<SweepReport> {
    return sendJson<SweepReport>("/api/inbox/check", {
      method: "POST",
      code: "CHECK_INBOX_FAILED",
      action: "check the inbox",
    });
  },

  async listMissions(this: void): Promise<Mission[]> {
    return getJson<Mission[]>("/api/missions", {
      code: "LIST_MISSIONS_FAILED",
      action: "list missions",
    });
  },

  async getMission(this: void, id: string): Promise<Mission> {
    return getJson<Mission>(`/api/missions/${segment(id)}`, {
      code: "GET_MISSION_FAILED",
      action: "load the mission",
    });
  },

  async createMission(this: void, request: CreateMissionRequest): Promise<Mission> {
    return sendJson<Mission>("/api/missions", {
      method: "POST",
      body: request,
      code: "CREATE_MISSION_FAILED",
      action: "create the mission",
    });
  },

  async missionAction(
    this: void,
    id: string,
    action: MissionAction,
    body?: { reason?: string; changeRequest?: string; text?: string },
  ): Promise<Mission> {
    return sendJson<Mission>(`/api/missions/${segment(id)}/${action}`, {
      method: "POST",
      body: body ?? {},
      code: "MISSION_ACTION_FAILED",
      action: `${action} the mission`,
    });
  },

  async previewBranchNames(this: void, git: GitSettings): Promise<BranchPreview> {
    return sendJson<BranchPreview>("/api/config/branch-preview", {
      method: "POST",
      body: git,
      code: "BRANCH_PREVIEW_FAILED",
      action: "preview branch names",
    });
  },

  async setMissionAgents(this: void, id: string, agents: MissionAgents): Promise<Mission> {
    return sendJson<Mission>(`/api/missions/${segment(id)}/agents`, {
      method: "PUT",
      body: agents,
      code: "UPDATE_MISSION_FAILED",
      action: "update the mission",
    });
  },

  async setMissionBudget(this: void, id: string, budget: Partial<MissionBudget>): Promise<Mission> {
    return sendJson<Mission>(`/api/missions/${segment(id)}/budget`, {
      method: "PUT",
      body: budget,
      code: "UPDATE_MISSION_FAILED",
      action: "update the mission",
    });
  },

  async listInboxProposals(this: void, state?: string): Promise<InboxProposal[]> {
    return getJson<InboxProposal[]>(
      withQuery("/api/inbox/proposals", { state: state?.trim() || undefined }),
      { code: "LIST_INBOX_PROPOSALS_FAILED", action: "list inbox proposals" },
    );
  },

  async acceptInboxProposal(this: void, id: number): Promise<{ jobId: string }> {
    return sendJson<{ jobId: string }>(`/api/inbox/proposals/${id}/accept`, {
      method: "POST",
      code: "ACCEPT_INBOX_PROPOSAL_FAILED",
      action: "accept the proposal",
    });
  },

  async dismissInboxProposal(this: void, id: number): Promise<void> {
    await sendVoid(`/api/inbox/proposals/${id}/dismiss`, {
      method: "POST",
      code: "DISMISS_INBOX_PROPOSAL_FAILED",
      action: "dismiss the proposal",
    });
  },

  /* --- Files ----------------------------------------------------------------------------------- */

  /** The daemon's guarded `/ivy/local-file`; the proxy supplies its `token`, as it does the bearer. */
  async getLocalFilePreview(this: void, path: string): Promise<string> {
    if (!path.trim()) throw bridgeError("VALIDATION_ERROR", "No file path was given to preview");
    const response = await fetch(withQuery("/ivy/local-file", { path }));
    if (response.status === 401) {
      throw bridgeError(
        "UNAUTHENTICATED",
        "The daemon refused the credential for a local file read",
      );
    }
    if (!response.ok) {
      throw bridgeError("NOT_FOUND", `Tendril will not serve '${path}' (${response.status})`);
    }
    const contentType = (response.headers.get("content-type") ?? "").split(";")[0].trim();
    const isVideo = contentType.startsWith("video/");
    if (!contentType.startsWith("image/") && contentType !== "application/pdf" && !isVideo) {
      throw bridgeError("VALIDATION_ERROR", `'${path}' is not a previewable file`);
    }
    const blob = await response.blob();
    // A recording attached as evidence may be larger than an image; the evidence command caps it at 48 MB.
    if (blob.size > (isVideo ? 48 * 1024 * 1024 : MAX_PREVIEW_BYTES)) {
      throw bridgeError("VALIDATION_ERROR", `'${path}' is too large to preview`);
    }
    return blobToDataUrl(new Blob([blob], { type: contentType }));
  },

  /** A path is a file on the app's own disk; a browser attaches bytes through the method below. */
  async uploadChatAttachment(this: void): Promise<ChatAttachment> {
    throw unsupported("Attaching a file by path");
  },

  async uploadAttachmentBytes(
    this: void,
    fileName: string,
    dataBase64: string,
    sessionId?: string,
  ): Promise<ChatAttachment> {
    const name = fileName.trim();
    const bytes = decodeBase64(dataBase64.trim());
    if (bytes.length === 0) throw bridgeError("VALIDATION_ERROR", `'${name}' is empty`);
    if (bytes.length > MAX_ATTACHMENT_BYTES) {
      throw bridgeError(
        "VALIDATION_ERROR",
        `'${name}' is larger than ${MAX_ATTACHMENT_BYTES / (1024 * 1024)} MiB and cannot be attached`,
      );
    }
    const session = sessionId?.trim() || UNASSIGNED_SESSION;
    const response = await fetch(
      withQuery(`/api/attachments/${segment(session)}`, { fileName: name }),
      {
        method: "POST",
        headers: { "Content-Type": "application/octet-stream" },
        body: bytes.buffer as ArrayBuffer,
      },
    );
    if (!response.ok) {
      const detail = await response.text().catch(() => "");
      throw bridgeError(
        "UPLOAD_ATTACHMENT_FAILED",
        `Tendril would not store '${name}' (${response.status}): ${detail}`,
      );
    }
    const stored = (await response.json()) as { path: string };
    return { name, path: stored.path };
  },
};

/**
 * Makes the app talk HTTP: installs `httpClient` behind `bridge`, and sends the page back to the
 * proxy's sign-in when a same-origin request comes back `401` - the session cookie expired or was
 * revoked, and every later call would fail the same way.
 */
export function installBrowserTransport(): void {
  setTendrilClient(httpClient);

  const nativeFetch = window.fetch.bind(window);
  let redirecting = false;
  window.fetch = async (input, init) => {
    const response = await nativeFetch(input, init);
    if (response.status === 401 && !redirecting) {
      const url = new URL(
        input instanceof Request ? input.url : String(input),
        window.location.href,
      );
      if (url.origin === window.location.origin) {
        redirecting = true;
        window.location.assign("/login");
      }
    }
    return response;
  };
}
