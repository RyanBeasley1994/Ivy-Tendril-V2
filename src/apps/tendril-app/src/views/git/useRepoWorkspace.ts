import React from "react";
import { bridge } from "../../api/bridge";
import { describeBridgeError } from "../../types/api";
import type { Graph, RepoDetail, RepoOp, RepoPrs } from "../../types/git";

export interface Notice {
  kind: "ok" | "error" | "conflict";
  message: string;
  conflicts: string[];
  /** The operation that produced it, so a conflict can say what stopped. */
  op?: string;
}

const PAGE = 300;
const POLL_MS = 10_000;

/** Everything the repository workspace shows, the way it is kept fresh, and the one way to change it. */
export function useRepoWorkspace(repoId: string) {
  const [detail, setDetail] = React.useState<RepoDetail | null>(null);
  const [graph, setGraph] = React.useState<Graph | null>(null);
  const [prs, setPrs] = React.useState<RepoPrs | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<Notice | null>(null);
  const [limit, setLimit] = React.useState(PAGE);
  const [branchFilter, setBranchFilter] = React.useState<string | null>(null);

  const generation = React.useRef(0);
  const noticeTimer = React.useRef<number | undefined>(undefined);
  const busyRef = React.useRef<string | null>(null);
  busyRef.current = busy;

  const refresh = React.useCallback(async () => {
    const mine = ++generation.current;
    try {
      const [d, g] = await Promise.all([
        bridge.gitRepo(repoId),
        bridge.gitGraph(repoId, limit, branchFilter ?? undefined),
      ]);
      // A newer refresh started while this one was in flight: its answer is the fresher one.
      if (mine !== generation.current) return;
      setDetail(d);
      setGraph(g);
      setError(null);
    } catch (e) {
      if (mine === generation.current) setError(describeBridgeError(e));
    }
  }, [repoId, limit, branchFilter]);

  const loadPrs = React.useCallback(async () => {
    try {
      setPrs(await bridge.gitPrs().then((all) => all[repoId] ?? { prs: [], error: null }));
    } catch {
      setPrs({ prs: [], error: "Pull requests could not be read." });
    }
  }, [repoId]);

  React.useEffect(() => {
    setDetail(null);
    setGraph(null);
    setPrs(null);
    setNotice(null);
    setLimit(PAGE);
    setBranchFilter(null);
  }, [repoId]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    void loadPrs();
  }, [loadPrs]);

  // The repository changes under us (an agent commits, a mission merges), so look again now and then,
  // but never while one of our own operations is running or the window is in the background.
  React.useEffect(() => {
    const timer = window.setInterval(() => {
      if (busyRef.current === null && document.visibilityState === "visible") void refresh();
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  /** Runs one operation; resolves true when it did what was asked. */
  const run = React.useCallback(
    async (op: RepoOp, label: string): Promise<boolean> => {
      setBusy(label);
      setNotice(null);
      let ok = false;
      try {
        const result = await bridge.gitOp(repoId, op);
        ok = result.outcome.ok;
        setNotice({
          kind: ok ? "ok" : result.outcome.conflicts.length > 0 ? "conflict" : "error",
          message: result.outcome.message,
          conflicts: result.outcome.conflicts,
          op: label,
        });
        // A success has told you what it had to; a problem stays until you dismiss it.
        window.clearTimeout(noticeTimer.current);
        if (ok) noticeTimer.current = window.setTimeout(() => setNotice((n) => (n?.kind === "ok" ? null : n)), 3500);
        if (ok && (op.op === "push" || op.op === "fetch")) void loadPrs();
      } catch (e) {
        setNotice({ kind: "error", message: describeBridgeError(e), conflicts: [], op: label });
      } finally {
        setBusy(null);
        await refresh();
      }
      return ok;
    },
    [repoId, refresh, loadPrs],
  );

  return {
    detail,
    graph,
    prs,
    error,
    busy,
    notice,
    dismissNotice: () => setNotice(null),
    branchFilter,
    setBranchFilter,
    loadMore: () => setLimit((l) => l + PAGE),
    refresh,
    loadPrs,
    run,
  };
}
