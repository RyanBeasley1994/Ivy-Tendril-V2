import { useSyncExternalStore } from "react";

/**
 * Which agent the sign-in dialog is open for, if any. A store rather than local state because the
 * dialog is opened from wherever an expired login shows up — a chat turn, the settings pane — and
 * mounted once in the App.
 */
let agent: string | null = null;
const listeners = new Set<() => void>();

const set = (next: string | null) => {
  agent = next;
  listeners.forEach((listener) => listener());
};

export const openAgentSignIn = (next: string) => set(next);
export const closeAgentSignIn = () => set(null);

export const useAgentSignIn = (): string | null =>
  useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => agent,
  );

/** The CLI behind a chat turn's agent id: Codex for anything Codex, Claude Code otherwise. */
export const signInAgentFor = (agentId?: string | null): string =>
  agentId?.toLowerCase().includes("codex") ? "codex" : "claude";

/** Whether a turn's text is a CLI saying its login has lapsed. */
export const isAuthFailure = (text: string): boolean =>
  /failed to authenticate|oauth (session|token) (has )?expired|not logged in|please run \/login|run `?claude (auth )?login|invalid api key|authentication_error|token has expired/i.test(
    text,
  );
