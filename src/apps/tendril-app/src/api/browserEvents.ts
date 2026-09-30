/**
 * The browser's stand-in for the Tauri events the native bridges emit.
 *
 * Under Tauri, `service/ws_bridge.rs` reads the daemon's `/api/ws` socket and `changes_bridge.rs` its
 * `/api/changes/events` stream, and both re-emit what they read as Tauri events, because the streams
 * are bearer-authenticated and the secret is native-only. A browser served by the authenticating
 * proxy can read both itself - same-origin, with the proxy adding the credential - so this module
 * opens them directly and hands the frames to the same channel names, routed the way
 * `route_ws_message` routes them. `events.ts` picks this or Tauri's `listen`, so no subscriber changes.
 *
 * Each stream opens with its first listener and stays open for the page's lifetime; the shell
 * subscribes once at startup, so there is no churn worth reference-counting a teardown for.
 *
 * The per-session streams (job events, review actions, the agent terminal) are not here: outside
 * Tauri their callers already read those routes directly (`subscribeToJobStream`,
 * `startReviewActionViaHttp`, `startViaHttp`), so their bridged channels simply never fire.
 */

type Handler<T> = (event: { payload: T }) => void;

const WS_CHANNELS = new Set(["service-status", "job-event", "plan-event", "chat-event"]);
const CHANGE_CHANNELS = new Set(["change-event", "change-stream-status"]);

const listeners = new Map<string, Set<Handler<unknown>>>();

function emit(channel: string, payload: unknown): void {
  for (const handler of listeners.get(channel) ?? []) {
    try {
      handler({ payload });
    } catch (err) {
      console.error(`[tendril] ${channel} listener failed`, err);
    }
  }
}

/** `route_ws_message`: which channel a daemon WebSocket message belongs on. */
export function routeWsMessage(text: string): [string, unknown] {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return ["job-event", text];
  }
  const type =
    value && typeof value === "object" && typeof (value as { type?: unknown }).type === "string"
      ? (value as { type: string }).type
      : "";
  if (type.startsWith("chat.")) return ["chat-event", value];
  if (
    type === "state" ||
    type === "status" ||
    type === "pr_status_changed" ||
    type.startsWith("plan.")
  ) {
    return ["plan-event", value];
  }
  return ["job-event", value];
}

function sameOriginUrl(path: string, protocol: "ws" | "http"): string {
  const url = new URL(path, window.location.href);
  if (protocol === "ws") url.protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

let wsStarted = false;

/** 500 ms doubling to 10 s, the ladder `WsBridge` and `ChangeBridge` both climb. */
function startWebSocket(): void {
  if (wsStarted) return;
  wsStarted = true;
  let backoff = 500;

  const connect = () => {
    emit("service-status", "reconnecting");
    const socket = new WebSocket(sameOriginUrl("/api/ws", "ws"));
    socket.onopen = () => {
      backoff = 500;
      emit("service-status", "connected");
    };
    socket.onmessage = (message) => {
      if (typeof message.data !== "string") return;
      const [channel, payload] = routeWsMessage(message.data);
      emit(channel, payload);
    };
    socket.onclose = () => {
      emit("service-status", "disconnected");
      window.setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 10_000);
    };
  };

  connect();
}

let changesStarted = false;

/** `EventSource` reconnects on its own; this only reports the transitions, as `ChangeBridge` does. */
function startChangeStream(): void {
  if (changesStarted) return;
  changesStarted = true;
  let connected = false;
  const setConnected = (next: boolean) => {
    if (next === connected) return;
    connected = next;
    emit("change-stream-status", next ? "connected" : "disconnected");
  };

  const source = new EventSource(sameOriginUrl("/api/changes/events", "http"));
  source.onopen = () => setConnected(true);
  source.onerror = () => setConnected(false);
  source.addEventListener("change", (event) => {
    try {
      emit("change-event", JSON.parse((event as MessageEvent<string>).data));
    } catch (err) {
      console.warn("[tendril] Discarding unparseable change frame", err);
    }
  });
}

export function listenInBrowser<T>(channel: string, handler: Handler<T>): Promise<() => void> {
  let set = listeners.get(channel);
  if (!set) {
    set = new Set();
    listeners.set(channel, set);
  }
  set.add(handler as Handler<unknown>);

  if (WS_CHANNELS.has(channel)) startWebSocket();
  if (CHANGE_CHANNELS.has(channel)) startChangeStream();

  return Promise.resolve(() => {
    set.delete(handler as Handler<unknown>);
  });
}
