import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "../utils/tauri";

/**
 * Propriotec Operations' support tickets, through the daemon's `/api/support` (which signs in to
 * Operations as the operator). Shapes are Operations' own `/api/team/tickets` responses, plus
 * `handoff` where Tendril handed the ticket to a project's manager.
 */

export type TicketStatus = "PENDING" | "OPEN" | "IN_PROGRESS" | "AWAITING_REPLY" | "RESOLVED" | "CLOSED";
export type TicketPriority = "CRITICAL" | "HIGH" | "MEDIUM" | "LOW";
export type TicketType = "CRITICAL" | "ISSUE" | "REQUEST" | "QUESTION";

export interface SupportStatus {
  configured: boolean;
  baseUrl: string;
  email: string;
  userName?: string | null;
}

export interface Handoff {
  ticketId: string;
  project: string;
  /** The ticket's own manager conversation. */
  sessionId: string;
  at: string;
  doneAt?: string | null;
}

export interface TicketSummary {
  id: string;
  subject: string;
  platform: string;
  priority: TicketPriority;
  /** Operations' classification; the By type board's columns. */
  type?: TicketType;
  status: TicketStatus;
  createdAt: string;
  /** The partner spoke last, or no one on the team has replied yet. */
  needsReply?: boolean;
  waitingSince?: string;
  updatedAt: string;
  partner?: { id: string; name: string } | null;
  assignedTo?: { id: string; name: string } | null;
  _count?: { messages: number };
  lastMessage?: { content: string; authorName: string; authorType: string } | null;
  hasUnread?: boolean;
  handoff?: Handoff;
  triage?: TriageSummary | null;
}

export type TriageCategory = "bug" | "question" | "request";
export type TriageSeverity = "critical" | "high" | "normal" | "low";

export interface TriageSummary {
  category?: TriageCategory | null;
  severity?: TriageSeverity | null;
  confidence?: number | null;
  pendingDrafts: number;
  auto: boolean;
}

export type ReplyKind = "ack" | "answer" | "resolved";

export interface TriageDraft {
  id: string;
  kind: ReplyKind;
  content: string;
  close: boolean;
  fixPr?: string;
  test?: string;
  createdAt: string;
  state: "pending" | "sent" | "discarded";
  decidedAt?: string;
}

export interface TriageCase {
  ticketId: string;
  triagedAt?: string | null;
  lookups: { at: string; source: string; request: string; result: string }[];
  investigation?: {
    at: string;
    category: TriageCategory;
    severity: TriageSeverity;
    confidence: number;
    findings: string;
    evidence: string[];
    duplicateOf?: string;
  } | null;
  drafts: TriageDraft[];
}

export interface DataSourceView {
  name: string;
  kind: "mongo" | "http";
  url: string;
  hasHeader: boolean;
  notes: string;
}

export interface TriageSettings {
  managerProject?: string | null;
  autoTriage: boolean;
  replyMode: "approve" | "auto";
  sources: DataSourceView[];
}

export interface DataSourceInput {
  name: string;
  kind: "mongo" | "http";
  url: string;
  /** Blank keeps the saved header; "-" removes it. */
  header?: string;
  notes: string;
}

export interface TicketMessage {
  id: string;
  authorType: string;
  authorName: string;
  content: string;
  isInternal: boolean;
  createdAt: string;
  attachments?: { url: string; filename: string; type?: string }[] | null;
}

export interface TicketDetail extends TicketSummary {
  messages: TicketMessage[];
  intakeData?: Record<string, string> | null;
  slaResponseBreached?: boolean;
  slaResolveBreached?: boolean;
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  if (isTauri()) return invoke<T>("cmd_support", { method, path, body: body ?? null });
  const res = await fetch(path, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error((data as { error?: string } | null)?.error ?? `${path} failed (${res.status})`);
  return data as T;
}

const ticket = (id: string, rest = "") => `/api/support/tickets/${encodeURIComponent(id)}${rest}`;

export const supportApi = {
  status: () => call<SupportStatus>("GET", "/api/support/status"),
  connect: (baseUrl: string, email: string, password: string) =>
    call<SupportStatus>("PUT", "/api/support/connection", { baseUrl, email, password }),
  disconnect: () => call<SupportStatus>("DELETE", "/api/support/connection"),
  count: () => call<{ count: number; configured: boolean }>("GET", "/api/support/count"),
  list: (includeDone: boolean) =>
    call<{ tickets: TicketSummary[]; total: number }>("GET", `/api/support/tickets?includeDone=${includeDone}`),
  get: (id: string) => call<TicketDetail>("GET", ticket(id)),
  markRead: (id: string) => call<unknown>("POST", ticket(id, "/read")),
  reply: (id: string, content: string, internal: boolean) =>
    call<unknown>("POST", ticket(id, "/reply"), { content, internal }),
  setStatus: (id: string, status: TicketStatus) => call<unknown>("POST", ticket(id, "/status"), { status }),
  setType: (id: string, type: TicketType) =>
    call<{ type: TicketType; priority: TicketPriority }>("POST", ticket(id, "/type"), { type }),
  close: (id: string, message?: string) => call<unknown>("POST", ticket(id, "/close"), { message }),
  handoff: (id: string, project: string, instructions: string) =>
    call<Handoff>("POST", ticket(id, "/handoff"), { project, instructions }),
  settings: () => call<TriageSettings>("GET", "/api/support/settings"),
  saveSettings: (s: { managerProject: string | null; autoTriage: boolean; replyMode: "approve" | "auto"; sources: DataSourceInput[] }) =>
    call<TriageSettings>("PUT", "/api/support/settings", s),
  triageCase: (id: string) => call<TriageCase>("GET", ticket(id, "/case")),
  approveDraft: (id: string, draft: string, content?: string) =>
    call<TriageDraft>("POST", ticket(id, `/drafts/${encodeURIComponent(draft)}`), { content }),
  discardDraft: (id: string, draft: string) => call<TriageDraft>("DELETE", ticket(id, `/drafts/${encodeURIComponent(draft)}`)),
};
