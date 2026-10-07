import { invoke } from "@tauri-apps/api/core";
import type { ChatAttachment, ChatMessage, ChatQueuedItem, ChatSession } from "../types/chat";
import { isTauri } from "../utils/tauri";

/* --- Browser transport ---------------------------------------------------------------------------
   The daemon's chat routes, called the way `service/client/chat.rs` calls them. Its answers are
   normalized to the shape the desktop app's DTOs (`ChatSessionDto` and friends in `models.rs`)
   serialize: those accept the V1 PascalCase spellings as aliases, so a record the daemon stored under
   either casing comes out camelCase here too. */

type Json = Record<string, unknown>;

/** The camelCase key, or its PascalCase alias. */
function field(source: unknown, key: string): unknown {
  const record = (source ?? {}) as Json;
  return record[key] ?? record[key[0].toUpperCase() + key.slice(1)];
}

function text(source: unknown, key: string): string | undefined {
  const value = field(source, key);
  return typeof value === "string" ? value : undefined;
}

function toAttachment(value: unknown): ChatAttachment {
  return {
    name: text(value, "name") ?? "",
    path: text(value, "path") ?? "",
    mimeType: text(value, "mimeType"),
  };
}

function toAttachments(value: unknown): ChatAttachment[] | undefined {
  const list = field(value, "attachments");
  return Array.isArray(list) ? list.map(toAttachment) : undefined;
}

function toMessage(value: unknown): ChatMessage {
  return {
    id: text(value, "id") ?? "",
    role: (text(value, "role") ?? "user") as ChatMessage["role"],
    content: text(value, "content") ?? "",
    timestamp: text(value, "timestamp") ?? "",
    agentId: text(value, "agentId"),
    modelId: text(value, "modelId"),
    rawStream: text(value, "rawStream"),
    effort: text(value, "effort"),
    attachments: toAttachments(value),
  };
}

function toSession(value: unknown): ChatSession {
  const messages = field(value, "messages");
  const spawned = field(value, "spawnedJobIds");
  const pinned = field(value, "isPinned");
  return {
    id: text(value, "id") ?? "",
    title: text(value, "title") ?? "",
    createdAt: text(value, "createdAt") ?? "",
    updatedAt: text(value, "updatedAt") ?? "",
    agentId: text(value, "agentId"),
    modelId: text(value, "modelId"),
    messages: Array.isArray(messages) ? messages.map(toMessage) : [],
    effort: text(value, "effort"),
    spawnedJobIds: Array.isArray(spawned)
      ? spawned.filter((id): id is string => typeof id === "string")
      : [],
    planFolderName: text(value, "planFolderName"),
    totalMessages: typeof field(value, "totalMessages") === "number" ? (field(value, "totalMessages") as number) : undefined,
    isPinned: typeof pinned === "boolean" ? pinned : undefined,
    pinnedAt: text(value, "pinnedAt"),
  };
}

function toQueuedItem(value: unknown): ChatQueuedItem {
  return {
    id: text(value, "id") ?? "",
    prompt: text(value, "prompt") ?? "",
    attachments: toAttachments(value),
    createdAt: text(value, "createdAt") ?? "",
    role: text(value, "role"),
  };
}

function sessionPath(id: string, suffix = ""): string {
  return `/api/chat/sessions/${encodeURIComponent(id)}${suffix}`;
}

/** Rejects the way a failed command does: a `BridgeError` carrying the daemon's own `error`. */
async function http(path: string, code: string, method = "GET", body?: unknown): Promise<Response> {
  let response: Response;
  try {
    response = await fetch(
      path,
      body === undefined
        ? { method }
        : { method, headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) },
    );
  } catch (err) {
    throw { code: "DISCONNECTED", message: "Tendril is not reachable", details: String(err) };
  }
  if (!response.ok) {
    const details = await response.text().catch(() => "");
    let message = `Chat request failed (${response.status})`;
    try {
      const parsed = JSON.parse(details) as { error?: unknown };
      if (typeof parsed.error === "string") message = parsed.error;
    } catch {
      // Not JSON; the status is the message.
    }
    throw { code, message, details };
  }
  return response;
}

async function httpJson(
  path: string,
  code: string,
  method = "GET",
  body?: unknown,
): Promise<unknown> {
  return (await http(path, code, method, body)).json();
}

export const chatApi = {
  /**
   * `summary` sends each conversation's latest message only (and `totalMessages`): enough for a list,
   * and the difference between kilobytes and megabytes once conversations get long.
   */
  async listSessions(summary = false): Promise<ChatSession[]> {
    if (!isTauri()) {
      const sessions = await httpJson(
        `/api/chat/sessions${summary ? "?summary=true" : ""}`,
        "LIST_CHAT_SESSIONS_FAILED",
      );
      return Array.isArray(sessions) ? sessions.map(toSession) : [];
    }
    return (await invoke<unknown[]>("cmd_list_chat_sessions", { summary })).map(toSession);
  },

  async createSession(args?: {
    title?: string;
    agentId?: string;
    modelId?: string;
    effort?: string;
    /**
     * The plan folder this session belongs to — `00021-BuildDesktopOperator`.
     *
     * V1's `PlanChatSessions.CreateForPlan` passes it, and `BelongsTo` is defined on it: "A session
     * belongs to exactly one plan, recorded on the session itself." It is what makes the plan page find
     * its own conversation again, so a plan chat's first message has to send it.
     */
    planFolderName?: string;
  }): Promise<ChatSession> {
    if (!isTauri()) {
      return toSession(
        await httpJson("/api/chat/sessions", "CREATE_CHAT_SESSION_FAILED", "POST", args ?? {}),
      );
    }
    return await invoke<ChatSession>("cmd_create_chat_session", { req: args });
  },

  /** `tail` fetches only the newest that many messages; `totalMessages` then says how many exist. */
  async getSession(id: string, tail?: number): Promise<ChatSession> {
    if (!isTauri()) {
      const query = tail ? `?tail=${tail}` : "";
      return toSession(await httpJson(sessionPath(id) + query, "GET_CHAT_SESSION_FAILED"));
    }
    return toSession(await invoke<unknown>("cmd_get_chat_session", { id, tail: tail ?? null }));
  },

  /** The page of messages before `before`, oldest first, and whether more precede it. */
  async getEarlierMessages(
    id: string,
    before: string,
    limit = 50,
  ): Promise<{ messages: ChatMessage[]; hasMore: boolean }> {
    const raw = !isTauri()
      ? await httpJson(
          `${sessionPath(id, "/messages")}?before=${encodeURIComponent(before)}&limit=${limit}`,
          "GET_EARLIER_CHAT_MESSAGES_FAILED",
        )
      : await invoke<unknown>("cmd_get_earlier_chat_messages", { id, before, limit });
    const messages = field(raw, "messages");
    return {
      messages: Array.isArray(messages) ? messages.map(toMessage) : [],
      hasMore: field(raw, "hasMore") === true,
    };
  },

  async updateSession(id: string, title: string): Promise<ChatSession> {
    if (!isTauri()) {
      return toSession(
        await httpJson(sessionPath(id), "UPDATE_CHAT_SESSION_FAILED", "PUT", { title }),
      );
    }
    return await invoke<ChatSession>("cmd_update_chat_session", { id, title });
  },

  async deleteSession(id: string): Promise<void> {
    if (!isTauri()) {
      await http(sessionPath(id), "DELETE_CHAT_SESSION_FAILED", "DELETE");
      return;
    }
    await invoke("cmd_delete_chat_session", { id });
  },

  async postMessage(
    id: string,
    prompt: string,
    options?: {
      enqueue?: boolean;
      attachments?: ChatAttachment[];
      role?: string;
    },
  ): Promise<{ started?: boolean; queued?: boolean; id?: string }> {
    if (!isTauri()) {
      return (await httpJson(sessionPath(id, "/messages"), "POST_CHAT_MESSAGE_FAILED", "POST", {
        prompt,
        enqueue: options?.enqueue,
        attachments: options?.attachments,
        role: options?.role,
      })) as { started?: boolean; queued?: boolean; id?: string };
    }
    return await invoke<{ started?: boolean; queued?: boolean; id?: string }>(
      "cmd_post_chat_message",
      {
        id,
        req: {
          prompt,
          enqueue: options?.enqueue,
          attachments: options?.attachments,
          role: options?.role,
        },
      },
    );
  },

  async executeTurn(
    id: string,
    options?: {
      prompt?: string;
      agentId?: string;
      modelId?: string;
      effort?: string;
    },
  ): Promise<void> {
    if (!isTauri()) {
      await http(sessionPath(id, "/execute"), "EXECUTE_CHAT_TURN_FAILED", "POST", options ?? {});
      return;
    }
    await invoke("cmd_execute_chat_turn", { id, req: options });
  },

  async cancelTurn(id: string): Promise<{ cancelled: boolean }> {
    if (!isTauri()) {
      const body = (await httpJson(
        sessionPath(id, "/cancel"),
        "CANCEL_CHAT_TURN_FAILED",
        "POST",
      )) as {
        cancelled?: unknown;
      };
      return { cancelled: typeof body?.cancelled === "boolean" ? body.cancelled : true };
    }
    const cancelled = await invoke<boolean>("cmd_cancel_chat_turn", { id });
    return { cancelled };
  },

  async answerQuestions(
    sessionId: string,
    messageId: string,
    answers: Record<string, string[]>,
  ): Promise<ChatSession> {
    if (!isTauri()) {
      const path = sessionPath(sessionId, `/messages/${encodeURIComponent(messageId)}/answers`);
      return toSession(await httpJson(path, "ANSWER_CHAT_QUESTIONS_FAILED", "POST", { answers }));
    }
    return await invoke<ChatSession>("cmd_answer_chat_questions", {
      sessionId,
      messageId,
      answers,
    });
  },

  async getQueue(id: string): Promise<ChatQueuedItem[]> {
    if (!isTauri()) {
      const items = await httpJson(sessionPath(id, "/queue"), "GET_CHAT_QUEUE_FAILED");
      return Array.isArray(items) ? items.map(toQueuedItem) : [];
    }
    return await invoke<ChatQueuedItem[]>("cmd_get_chat_queue", { id });
  },

  async enqueueItem(
    id: string,
    prompt: string,
    attachments?: ChatAttachment[],
  ): Promise<ChatQueuedItem> {
    if (!isTauri()) {
      const item = await httpJson(
        sessionPath(id, "/queue"),
        "ENQUEUE_CHAT_MESSAGE_FAILED",
        "POST",
        {
          prompt,
          attachments,
        },
      );
      return toQueuedItem(item);
    }
    return await invoke<ChatQueuedItem>("cmd_enqueue_chat_message", {
      id,
      req: { prompt, attachments },
    });
  },

  async clearQueue(id: string): Promise<void> {
    if (!isTauri()) {
      await http(sessionPath(id, "/queue"), "CLEAR_CHAT_QUEUE_FAILED", "DELETE");
      return;
    }
    await invoke("cmd_clear_chat_queue", { id });
  },

  async deleteQueuedItem(sessionId: string, itemId: string): Promise<void> {
    if (!isTauri()) {
      const path = sessionPath(sessionId, `/queue/${encodeURIComponent(itemId)}`);
      await http(path, "DELETE_QUEUED_CHAT_ITEM_FAILED", "DELETE");
      return;
    }
    await invoke("cmd_delete_queued_chat_item", { sessionId, itemId });
  },

  /** Rewrites a queued prompt in place, so an edit keeps its position in the queue. */
  async updateQueuedItem(
    sessionId: string,
    itemId: string,
    prompt: string,
  ): Promise<ChatQueuedItem> {
    if (!isTauri()) {
      const path = sessionPath(sessionId, `/queue/${encodeURIComponent(itemId)}`);
      return toQueuedItem(
        await httpJson(path, "UPDATE_QUEUED_CHAT_ITEM_FAILED", "PUT", { prompt }),
      );
    }
    return await invoke<ChatQueuedItem>("cmd_update_queued_chat_item", {
      sessionId,
      itemId,
      prompt,
    });
  },
};
