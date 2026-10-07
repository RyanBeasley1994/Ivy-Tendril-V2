import { bridge } from "../api/bridge";
import { chatApi } from "../api/chatApi";
import { jobsStore } from "./jobsStore";
import { navigation } from "./navigation";
import { notificationsStore } from "./notificationsStore";
import { projectStatus } from "../utils/projectStatus";

/**
 * The only things that notify: a manager that has stopped and needs you, and a manager whose work is
 * complete. A single mission or task finishing says nothing, because the manager is the one who has
 * to care about it.
 *
 * A manager's turn ending is the signal. When it ends:
 * - it asked something, or a mission is waiting on you -> "needs you"
 * - nothing is left running for the project -> "work complete"
 * - workers are still going and it asked nothing -> silence; it is waiting on them, not on you
 */
const POLL_MS = 4_000;

const plain = (text: string): string =>
  text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/[`*_#>]/g, "")
    .replace(/\s+/g, " ")
    .trim();

const lastParagraph = (text: string): string => text.trim().split(/\n\s*\n/).pop()?.trim() ?? "";

const clip = (text: string, max = 140): string => (text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text);

const managerSessionId = (project: string): string =>
  `manager-${[...project].map((c) => (/[A-Za-z0-9]/.test(c) ? c.toLowerCase() : "-")).join("")}`;

/** Whether the person is already looking at this project, in which case nothing needs announcing. */
const isWatching = (project: string): boolean =>
  typeof document !== "undefined" && document.hasFocus() && navigation.getState().pageAppId === `project-${project}`;

export const startManagerNotifier = (): (() => void) => {
  let stopped = false;
  let baseline = false;
  const wasWorking = new Map<string, boolean>();
  const lastKey = new Map<string, string>();

  const onTurnEnded = async (project: string) => {
    const [missions, session] = await Promise.all([
      bridge.listMissions().catch(() => []),
      chatApi.getSession(managerSessionId(project), 12).catch(() => null),
    ]);
    const status = projectStatus(project, missions, jobsStore.getState().jobs);
    const reply = [...(session?.messages ?? [])].reverse().find((m) => m.role === "assistant" && m.content.trim());
    const text = reply ? plain(reply.content) : "";
    const asked = reply ? lastParagraph(reply.content).endsWith("?") : false;
    // Retries and re-plans are the manager's to handle; only a mission waiting on a person counts.
    const waiting = status.missions.find(
      (m) => m.state === "AwaitingApproval" || m.state === "Paused",
    );
    const blocked = waiting !== undefined;
    const stillWorking = status.live.some((m) => m.state === "Running" || m.state === "Planning" || m.state === "Validating") || status.runningJobs.length > 0;

    let kind: "needs" | "done" | null = null;
    if (asked || blocked) kind = "needs";
    else if (!stillWorking) kind = "done";
    if (!kind || isWatching(project)) return;

    // The same outcome twice in a row (a re-poll, a quick follow-up turn) is one notification.
    const key = `${kind}:${reply?.id ?? ""}`;
    if (lastKey.get(project) === key) return;
    lastKey.set(project, key);

    const body =
      kind === "needs"
        ? clip(
            asked
              ? lastParagraph(plain(reply?.content ?? "")) || text
              : `"${waiting?.title}" is ${waiting?.state === "Paused" ? "paused" : "waiting for approval"}.`,
          )
        : clip(text || status.headline);
    notificationsStore.notifyManager({
      title: kind === "needs" ? `${project} · needs you` : `${project} · work complete`,
      message: body || (kind === "needs" ? "The manager is waiting on you." : "The manager has finished."),
      isSuccess: kind === "done",
    });
  };

  const tick = async () => {
    try {
      const status = await bridge.getManagersStatus();
      if (stopped) return;
      for (const [project, { working }] of Object.entries(status)) {
        const before = wasWorking.get(project);
        wasWorking.set(project, working);
        // The first snapshot only records where things stand: a manager already mid-turn when the app
        // opens must not announce a turn it did not see start.
        if (baseline && before === true && working === false) void onTurnEnded(project);
      }
      baseline = true;
    } catch {
      /* daemon unreachable: try again next tick */
    }
  };

  void tick();
  const timer = window.setInterval(tick, POLL_MS);
  return () => {
    stopped = true;
    window.clearInterval(timer);
  };
};
