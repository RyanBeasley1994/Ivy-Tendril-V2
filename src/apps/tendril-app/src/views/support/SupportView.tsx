import React from "react";
import { Bot, LifeBuoy, Lock, RefreshCw, Send, X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { ProjectSummary } from "../../types/api";
import {
  supportApi,
  type SupportStatus,
  type TicketDetail,
  type TicketPriority,
  type TicketStatus,
  type TicketSummary,
} from "../../api/support";
import { refreshSupportCount } from "../../state/supportCount";
import { chatStore } from "../../state/chatStore";
import type { ChatMessage } from "../../types/chat";
import { ChatView } from "../ChatView";
import { formatAge } from "../../utils/commandCenter";
import { Page, PageHeader } from "../../components/page/Page";
import { Card, GhostButton, IconButton, Pill, PrimaryButton, Seg, type Tone } from "../../components/page/kit";

/**
 * Propriotec Operations' support board, worked from Tendril: the same five lanes as the Operations
 * Kanban, a ticket panel that reads and replies as the operator, and a hand-off that gives a ticket
 * to a project's manager without the partner seeing any of it.
 */

type Lane = "OPEN" | "IN_PROGRESS" | "AWAITING_REPLY" | "PENDING" | "RESOLVED";

/** Operations' `COLUMNS`, in its order and with its names. */
const LANES: { key: Lane; label: string; statuses: TicketStatus[] }[] = [
  { key: "OPEN", label: "Needs Us", statuses: ["OPEN"] },
  { key: "IN_PROGRESS", label: "In Progress", statuses: ["IN_PROGRESS"] },
  { key: "AWAITING_REPLY", label: "Awaiting Reply", statuses: ["AWAITING_REPLY"] },
  { key: "PENDING", label: "Can Wait", statuses: ["PENDING"] },
  { key: "RESOLVED", label: "Done", statuses: ["RESOLVED", "CLOSED"] },
];

const laneOf = (status: TicketStatus): Lane => LANES.find((l) => l.statuses.includes(status))?.key ?? "OPEN";

const PRIORITY_TONE: Record<TicketPriority, Tone> = { CRITICAL: "bad", HIGH: "warn", MEDIUM: "info", LOW: "mute" };
const PRIORITY_ORDER: Record<TicketPriority, number> = { CRITICAL: 0, HIGH: 1, MEDIUM: 2, LOW: 3 };

const ago = (iso: string) => formatAge(Math.max(0, Date.now() - new Date(iso).getTime()));
const errorText = (e: unknown) =>
  e instanceof Error ? e.message : typeof e === "object" && e && "message" in e ? String(e.message) : String(e);

export const SupportView: React.FC<{ projects: ProjectSummary[] }> = ({ projects }) => {
  const [status, setStatus] = React.useState<SupportStatus | null>(null);
  React.useEffect(() => {
    supportApi.status().then(setStatus).catch(() => setStatus({ configured: false, baseUrl: "", email: "" }));
  }, []);

  if (!status) return <Page><span className="text-[13px] text-muted-foreground">Loading…</span></Page>;
  if (!status.configured) return <ConnectPage initial={status} onConnected={setStatus} />;
  return <Board status={status} projects={projects} onDisconnected={setStatus} />;
};

/* ------------------------------------------------------------------ connect */

const inputClass =
  "h-8 w-full rounded-md border border-input bg-muted px-2.5 text-[13px] text-foreground outline-none focus:border-primary/60";

const ConnectPage: React.FC<{ initial: SupportStatus; onConnected: (s: SupportStatus) => void }> = ({
  initial,
  onConnected,
}) => {
  const [baseUrl, setBaseUrl] = React.useState(initial.baseUrl || "https://portal.propriotec.com");
  const [email, setEmail] = React.useState(initial.email);
  const [password, setPassword] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onConnected(await supportApi.connect(baseUrl, email, password));
      void refreshSupportCount();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Page>
      <PageHeader title="Support" subtitle="Work Propriotec Operations tickets here, and hand them to a project's manager." />
      <Card title="Connect to Operations" className="max-w-[460px]" bodyClassName="p-4">
        <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-3">
          <p className="m-0 text-[12.5px] text-muted-foreground">
            Sign in with your own Operations account (ADMIN or SUPPORT). Tendril reads tickets as you, so your
            unread count and notifications clear here just as they do in Operations. The password stays on the
            Tendril server.
          </p>
          <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
            Operations URL
            <input className={cn(inputClass, "font-mono text-[12px]")} value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
          </label>
          <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
            Email
            <input className={inputClass} type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} />
          </label>
          <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
            Password
            <input
              className={inputClass}
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
          {error && <span className="text-[12px] text-destructive">{error}</span>}
          <PrimaryButton type="submit" className="self-start" disabled={busy || !email || !password}>
            {busy ? "Signing in…" : "Sign in"}
          </PrimaryButton>
        </form>
      </Card>
    </Page>
  );
};

/* ------------------------------------------------------------------ board */

const Board: React.FC<{
  status: SupportStatus;
  projects: ProjectSummary[];
  onDisconnected: (s: SupportStatus) => void;
}> = ({ status, projects, onDisconnected }) => {
  const [tickets, setTickets] = React.useState<TicketSummary[] | null>(null);
  const [showDone, setShowDone] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [openId, setOpenId] = React.useState<string | null>(null);
  const [dragOver, setDragOver] = React.useState<Lane | null>(null);
  const dragging = React.useRef<string | null>(null);

  const load = React.useCallback(async () => {
    try {
      const res = await supportApi.list(showDone);
      setTickets(res.tickets);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [showDone]);

  React.useEffect(() => {
    void load();
    // Operations' own board refetches every 15s.
    const timer = window.setInterval(() => void load(), 15_000);
    return () => window.clearInterval(timer);
  }, [load]);

  const move = async (id: string, lane: Lane) => {
    const current = tickets?.find((t) => t.id === id);
    if (!current || laneOf(current.status) === lane) return;
    const next: TicketStatus = lane;
    setTickets((all) => all?.map((t) => (t.id === id ? { ...t, status: next } : t)) ?? null);
    try {
      await supportApi.setStatus(id, next);
      void refreshSupportCount();
    } catch (e) {
      setError(errorText(e));
      void load();
    }
  };

  const lanes = showDone ? LANES : LANES.filter((l) => l.key !== "RESOLVED");
  const disconnect = async () => onDisconnected(await supportApi.disconnect());

  return (
    <div className="flex min-h-0 flex-1">
      <Page scroll={false} className="min-w-0">
        <PageHeader
          title="Support"
          subtitle={`Propriotec Operations · signed in as ${status.userName ?? status.email}`}
          actions={
            <>
              <GhostButton size="sm" aria-pressed={showDone} onClick={() => setShowDone((v) => !v)}>
                {showDone ? "Hide done" : "Show done"}
              </GhostButton>
              <IconButton label="Refresh" onClick={() => void load()}>
                <RefreshCw className="size-3.5" aria-hidden="true" />
              </IconButton>
              <GhostButton size="sm" onClick={() => void disconnect()} title="Forget the Operations login">
                Disconnect
              </GhostButton>
            </>
          }
        />
        {error && <div className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">{error}</div>}
        {tickets === null && !error && <span className="text-[13px] text-muted-foreground">Loading tickets…</span>}
        {tickets && (
          <div className="flex min-h-0 flex-1 gap-3 overflow-x-auto pb-1">
            {lanes.map((lane) => {
              const inLane = tickets
                .filter((t) => laneOf(t.status) === lane.key)
                .sort(
                  (a, b) =>
                    Number(!!b.hasUnread) - Number(!!a.hasUnread) ||
                    PRIORITY_ORDER[a.priority] - PRIORITY_ORDER[b.priority] ||
                    b.updatedAt.localeCompare(a.updatedAt),
                );
              return (
                <section
                  key={lane.key}
                  aria-label={lane.label}
                  onDragOver={(e) => {
                    e.preventDefault();
                    setDragOver(lane.key);
                  }}
                  onDragLeave={() => setDragOver((d) => (d === lane.key ? null : d))}
                  onDrop={(e) => {
                    e.preventDefault();
                    setDragOver(null);
                    if (dragging.current) void move(dragging.current, lane.key);
                    dragging.current = null;
                  }}
                  className={cn(
                    "flex w-[272px] shrink-0 flex-col rounded-xl border bg-card/40 transition-colors",
                    dragOver === lane.key ? "border-primary/50 bg-primary/5" : "border-border",
                  )}
                >
                  <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border/70 px-3">
                    <span className="text-[12.5px] font-semibold text-foreground">{lane.label}</span>
                    <span className="rounded-full bg-secondary px-1.5 font-mono text-[10.5px] text-muted-foreground">{inLane.length}</span>
                  </header>
                  <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2">
                    {inLane.length === 0 && <span className="px-1 py-3 text-center text-[11.5px] text-muted-foreground">Nothing here</span>}
                    {inLane.map((t) => (
                      <TicketCard
                        key={t.id}
                        ticket={t}
                        active={openId === t.id}
                        onOpen={() => setOpenId(t.id)}
                        onDragStart={() => (dragging.current = t.id)}
                      />
                    ))}
                  </div>
                </section>
              );
            })}
          </div>
        )}
      </Page>
      {openId && (
        <TicketPanel
          key={openId}
          id={openId}
          projects={projects}
          onClose={() => setOpenId(null)}
          onChanged={() => void load()}
        />
      )}
    </div>
  );
};

const TicketCard: React.FC<{ ticket: TicketSummary; active: boolean; onOpen: () => void; onDragStart: () => void }> = ({
  ticket: t,
  active,
  onOpen,
  onDragStart,
}) => (
  <button
    type="button"
    draggable
    onDragStart={onDragStart}
    onClick={onOpen}
    data-testid={`support-ticket-${t.id}`}
    className={cn(
      "flex flex-col gap-1.5 rounded-lg border bg-card p-2.5 text-left transition-colors hover:border-primary/40",
      active ? "border-primary/60" : "border-border",
    )}
  >
    <div className="flex items-center gap-1.5">
      <Pill tone={PRIORITY_TONE[t.priority]}>{t.priority.toLowerCase()}</Pill>
      {t.handoff && !t.handoff.doneAt && (
        <Pill tone="violet">
          <Bot className="size-3" aria-hidden="true" />
          {t.handoff.project}
        </Pill>
      )}
      <span className="ml-auto font-mono text-[10.5px] text-muted-foreground">{ago(t.updatedAt)}</span>
    </div>
    <span className={cn("text-[12.5px] leading-snug text-foreground", t.hasUnread ? "font-semibold" : "font-medium")}>
      {t.hasUnread && <span className="mr-1.5 inline-block size-1.5 translate-y-[-2px] rounded-full bg-primary" aria-label="Unread" />}
      {t.subject}
    </span>
    <span className="truncate text-[11px] text-muted-foreground">
      {t.partner?.name ?? "No partner"} · {t.platform.toLowerCase()}
      {t.assignedTo ? ` · ${t.assignedTo.name}` : ""}
    </span>
    {t.lastMessage && (
      <span className={cn("line-clamp-2 text-[11.5px]", t.hasUnread ? "text-foreground" : "text-muted-foreground")}>
        <b className="font-medium">{t.lastMessage.authorName}:</b> {t.lastMessage.content}
      </span>
    )}
  </button>
);

/* ------------------------------------------------------------------ ticket */

const STATUS_LABEL: Record<TicketStatus, string> = {
  OPEN: "Needs Us",
  IN_PROGRESS: "In Progress",
  AWAITING_REPLY: "Awaiting Reply",
  PENDING: "Can Wait",
  RESOLVED: "Resolved",
  CLOSED: "Closed",
};

const TicketPanel: React.FC<{
  id: string;
  projects: ProjectSummary[];
  onClose: () => void;
  onChanged: () => void;
}> = ({ id, projects, onClose, onChanged }) => {
  const [ticket, setTicket] = React.useState<TicketDetail | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [mode, setMode] = React.useState<"reply" | "close">("reply");
  const [view, setView] = React.useState<"ticket" | "manager">("ticket");
  const bottom = React.useRef<HTMLDivElement>(null);

  const load = React.useCallback(async () => {
    try {
      setTicket(await supportApi.get(id));
    } catch (e) {
      setError(errorText(e));
    }
  }, [id]);

  // Opening a ticket reads it, as the operator: Operations' read receipt and notifications.
  React.useEffect(() => {
    void load();
    supportApi
      .markRead(id)
      .then(() => {
        void refreshSupportCount();
        onChanged();
      })
      .catch(() => undefined);
    const timer = window.setInterval(() => void load(), 10_000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  const count = ticket?.messages.length ?? 0;
  React.useEffect(() => bottom.current?.scrollIntoView({ block: "end" }), [count]);

  const act = async (run: () => Promise<unknown>) => {
    setError(null);
    try {
      await run();
      await load();
      onChanged();
      void refreshSupportCount();
      // Replying is reading: the operator just saw everything up to their own message.
      void supportApi.markRead(id).catch(() => undefined);
    } catch (e) {
      setError(errorText(e));
      throw e;
    }
  };

  return (
    <aside
      className="fixed inset-0 z-30 flex flex-col border-l border-border bg-background lg:static lg:z-auto lg:w-[580px] lg:shrink-0"
      aria-label="Ticket"
    >
      <header className="flex shrink-0 items-start gap-2 border-b border-border/70 px-4 py-3">
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <h2 className="m-0 text-[15px] font-semibold leading-snug text-foreground">{ticket?.subject ?? "Loading…"}</h2>
          {ticket && (
            <div className="flex flex-wrap items-center gap-1.5 text-[11.5px] text-muted-foreground">
              <Pill tone={PRIORITY_TONE[ticket.priority]}>{ticket.priority.toLowerCase()}</Pill>
              <select
                value={ticket.status}
                onChange={(e) => void act(() => supportApi.setStatus(id, e.target.value as TicketStatus)).catch(() => undefined)}
                className="h-6 rounded-md border border-input bg-muted px-1.5 text-[11.5px] text-foreground"
                aria-label="Status"
              >
                {(Object.keys(STATUS_LABEL) as TicketStatus[]).map((s) => (
                  <option key={s} value={s}>{STATUS_LABEL[s]}</option>
                ))}
              </select>
              <span>{ticket.partner?.name ?? "No partner"}</span>
              <span>· {ticket.platform.toLowerCase()}</span>
              {(ticket.slaResponseBreached || ticket.slaResolveBreached) && <Pill tone="bad">SLA breached</Pill>}
              {ticket.handoff && (
                <Pill tone="violet">
                  <Bot className="size-3" aria-hidden="true" />
                  {ticket.handoff.doneAt ? `done by ${ticket.handoff.project}` : `with ${ticket.handoff.project}`}
                </Pill>
              )}
            </div>
          )}
        </div>
        <IconButton label="Close ticket panel" onClick={onClose}>
          <X className="size-4" aria-hidden="true" />
        </IconButton>
      </header>
      <div className="flex shrink-0 border-b border-border/70 px-4 py-2">
        <Seg<"ticket" | "manager">
          label="Show"
          value={view}
          onChange={setView}
          options={[
            { value: "ticket", label: "Ticket" },
            { value: "manager", label: ticket?.handoff ? `Manager · ${ticket.handoff.project}` : "Manager" },
          ]}
        />
      </div>

      {view === "manager" && ticket && (
        <ManagerPane
          ticket={ticket}
          projects={projects}
          onHandoff={(project, instructions) => act(() => supportApi.handoff(id, project, instructions))}
        />
      )}
      {view === "ticket" && (
      <>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto px-4 py-3">
        {ticket?.intakeData && Object.keys(ticket.intakeData).length > 0 && (
          <div className="rounded-lg border border-border bg-card px-3 py-2 text-[11.5px] text-muted-foreground">
            {Object.entries(ticket.intakeData)
              .filter(([, v]) => v)
              .map(([k, v]) => (
                <div key={k}><span className="text-foreground">{k}</span>: {String(v)}</div>
              ))}
          </div>
        )}
        {ticket?.messages.map((m) => {
          const partner = m.authorType === "PARTNER";
          return (
            <div
              key={m.id}
              className={cn(
                "flex max-w-[88%] flex-col gap-1 rounded-xl border px-3 py-2",
                m.isInternal
                  ? "self-stretch max-w-none border-warning/30 bg-warning/8"
                  : partner
                    ? "self-start border-border bg-card"
                    : "self-end border-primary/25 bg-primary/8",
              )}
            >
              <div className="flex items-center gap-1.5 text-[10.5px] text-muted-foreground">
                {m.isInternal && <Lock className="size-3 text-warning" aria-hidden="true" />}
                <span className="font-medium text-foreground">{m.authorName}</span>
                {m.isInternal && <span className="text-warning">internal</span>}
                <span className="ml-auto font-mono">{ago(m.createdAt)}</span>
              </div>
              <div className="whitespace-pre-wrap break-words text-[12.5px] text-foreground">{m.content}</div>
              {m.attachments?.map((a) => (
                <span key={a.url} className="truncate font-mono text-[11px] text-info">{a.filename}</span>
              ))}
            </div>
          );
        })}
        <div ref={bottom} />
      </div>

      {error && <div className="shrink-0 border-t border-destructive/30 bg-destructive/10 px-4 py-2 text-[12px] text-destructive">{error}</div>}

      {ticket && (
        <footer className="flex shrink-0 flex-col gap-2 border-t border-border/70 px-4 py-3">
          <div className="flex gap-1">
            {(
              [
                ["reply", "Reply"],
                ["close", "Close"],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                type="button"
                onClick={() => setMode(value)}
                aria-pressed={mode === value}
                className={cn(
                  "h-7 rounded-md px-2.5 text-[12px] transition-colors",
                  mode === value ? "bg-secondary text-foreground" : "text-muted-foreground hover:text-foreground",
                )}
              >
                {label}
              </button>
            ))}
          </div>
          {mode === "reply" && <Composer onSend={(text, internal) => act(() => supportApi.reply(id, text, internal))} />}
          {mode === "close" && (
            <CloseForm
              onClose={(message) =>
                act(() => supportApi.close(id, message)).then(() => {
                  onClose();
                })
              }
            />
          )}
        </footer>
      )}
      </>
      )}
    </aside>
  );
};

/** The briefing and the ticket dump that opens the conversation: instructions, shown in the Ticket view. */
const isTicketChatNote = (m: ChatMessage): boolean =>
  (m.role === "system" && m.content.startsWith("# You are the Factory Manager")) ||
  m.content.startsWith("# Support ticket handed to you");

const TICKET_PROMPTS = [
  { label: "What did you find?", prompt: "What have you found so far, and what's your plan?" },
  { label: "Reproduce it first", prompt: "Before fixing anything, have a worker reproduce exactly what the partner described and show me the evidence." },
  { label: "Close it out", prompt: "If the fix is merged, tell the partner it's going out shortly and close the ticket." },
];

/**
 * The ticket's own manager conversation, or the form that starts it. Once started it is an ordinary
 * manager chat the operator guides; the partner never sees it.
 */
const ManagerPane: React.FC<{
  ticket: TicketDetail;
  projects: ProjectSummary[];
  onHandoff: (project: string, instructions: string) => Promise<void>;
}> = ({ ticket, projects, onHandoff }) => {
  const sessionId = ticket.handoff?.sessionId;
  if (!sessionId) {
    return (
      <div className="flex min-h-0 flex-1 flex-col justify-end gap-2 px-4 py-3">
        <HandoffForm projects={projects} onHandoff={onHandoff} />
      </div>
    );
  }
  return <TicketChat sessionId={sessionId} ticket={ticket} />;
};

const TicketChat: React.FC<{ sessionId: string; ticket: TicketDetail }> = ({ sessionId, ticket }) => {
  const [ready, setReady] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    let live = true;
    setReady(false);
    (async () => {
      try {
        // `init` lists the sessions once; listing again on every open is the slow path.
        await chatStore.init();
        await chatStore.selectSession(sessionId);
        if (live) setReady(true);
      } catch (e) {
        if (live) setError(errorText(e));
      }
    })();
    return () => {
      live = false;
    };
  }, [sessionId]);
  if (error) return <div className="p-4 text-[12.5px] text-destructive">Could not open the conversation: {error}</div>;
  if (!ready) return <div className="p-4 text-[12.5px] text-muted-foreground">Opening the conversation…</div>;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ChatView
        embedded
        greeting={`Manager · ${ticket.handoff?.project ?? ""}`}
        headline={ticket.subject}
        hideMessage={isTicketChatNote}
        samplePrompts={TICKET_PROMPTS}
      />
    </div>
  );
};

const textareaClass =
  "min-h-[72px] w-full resize-y rounded-lg border border-input bg-muted px-2.5 py-2 text-[12.5px] text-foreground outline-none focus:border-primary/60";

const Composer: React.FC<{ onSend: (text: string, internal: boolean) => Promise<void> }> = ({ onSend }) => {
  const [text, setText] = React.useState("");
  const [internal, setInternal] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const send = async () => {
    if (!text.trim()) return;
    setBusy(true);
    try {
      await onSend(text, internal);
      setText("");
    } catch {
      // Shown by the panel.
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-col gap-2">
      <textarea
        className={cn(textareaClass, internal && "border-warning/40 bg-warning/5")}
        placeholder={internal ? "Internal note — only the team sees this" : "Reply to the partner"}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void send();
        }}
      />
      <div className="flex items-center gap-2">
        <label className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
          <input type="checkbox" checked={internal} onChange={(e) => setInternal(e.target.checked)} />
          <Lock className="size-3" aria-hidden="true" /> Internal note
        </label>
        <PrimaryButton size="sm" className="ml-auto" disabled={busy || !text.trim()} onClick={() => void send()}>
          <Send className="size-3.5" aria-hidden="true" />
          {internal ? "Add note" : "Send reply"}
        </PrimaryButton>
      </div>
    </div>
  );
};

const HandoffForm: React.FC<{
  projects: ProjectSummary[];
  onHandoff: (project: string, instructions: string) => Promise<void>;
}> = ({ projects, onHandoff }) => {
  const [project, setProject] = React.useState(projects[0]?.name ?? "");
  const [instructions, setInstructions] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const submit = async () => {
    setBusy(true);
    try {
      await onHandoff(project, instructions);
      setInstructions("");
    } catch {
      // Shown by the panel.
    } finally {
      setBusy(false);
    }
  };
  if (projects.length === 0) return <span className="text-[12px] text-muted-foreground">No projects to hand it to yet.</span>;
  return (
    <div className="flex flex-col gap-2">
      <p className="m-0 text-[12px] text-muted-foreground">
        Starts a fresh manager conversation about this ticket, on the project you pick. It gets the whole ticket, works it
        with agents and reports back here, where you guide it. When the fix is out it tells the partner and closes the
        ticket. The partner sees nothing until then.
      </p>
      <select
        value={project}
        onChange={(e) => setProject(e.target.value)}
        className="h-8 rounded-md border border-input bg-muted px-2 text-[12.5px] text-foreground"
        aria-label="Project"
      >
        {projects.map((p) => (
          <option key={p.name} value={p.name}>{p.name}</option>
        ))}
      </select>
      <textarea
        className={textareaClass}
        placeholder="Your first steer (optional): where to look, what to check, how far to take it"
        value={instructions}
        onChange={(e) => setInstructions(e.target.value)}
      />
      <PrimaryButton size="sm" className="self-end" disabled={busy || !project} onClick={() => void submit()}>
        <Bot className="size-3.5" aria-hidden="true" />
        {busy ? "Starting…" : "Start manager chat"}
      </PrimaryButton>
    </div>
  );
};

const CloseForm: React.FC<{ onClose: (message?: string) => Promise<void> }> = ({ onClose }) => {
  const [message, setMessage] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const submit = async () => {
    setBusy(true);
    try {
      await onClose(message.trim() || undefined);
    } catch {
      // Shown by the panel.
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-col gap-2">
      <textarea
        className={textareaClass}
        placeholder="Message to the partner before closing (optional). Leave empty to resolve silently."
        value={message}
        onChange={(e) => setMessage(e.target.value)}
      />
      <PrimaryButton size="sm" className="self-end" disabled={busy} onClick={() => void submit()}>
        <LifeBuoy className="size-3.5" aria-hidden="true" />
        {busy ? "Closing…" : message.trim() ? "Send and resolve" : "Resolve"}
      </PrimaryButton>
    </div>
  );
};
