import React from "react";
import { Check, Database, FlaskConical, Plus, Search, ShieldCheck, Trash2, X } from "lucide-react";
import {
  cn,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@ivy-interactive/components/ui";
import type { ProjectSummary } from "../../types/api";
import {
  supportApi,
  type DataSourceInput,
  type ReplyKind,
  type TriageCase,
  type TriageCategory,
  type TriageSeverity,
  type TriageSettings,
  type TriageSummary,
} from "../../api/support";
import { openUrl } from "../../utils/opener";
import { GhostButton, Pill, PrimaryButton, Seg, type Tone } from "../../components/page/kit";

/**
 * Support triage in the app: the settings that send new tickets to one manager, what that manager
 * found on a ticket, and the replies it is waiting for the operator to approve.
 */

const errorText = (e: unknown) =>
  e instanceof Error ? e.message : typeof e === "object" && e && "message" in e ? String(e.message) : String(e);

const inputClass =
  "h-8 w-full rounded-md border border-input bg-muted px-2.5 text-[12.5px] text-foreground outline-none focus:border-primary/60";

const SEVERITY_TONE: Record<TriageSeverity, Tone> = { critical: "bad", high: "warn", normal: "info", low: "mute" };
const CATEGORY_LABEL: Record<TriageCategory, string> = { bug: "bug", question: "question", request: "request" };
const KIND_LABEL: Record<ReplyKind, string> = { ack: "We're on it", answer: "Answer", resolved: "Resolved" };

/** The board card's triage pills. */
export const TriagePills: React.FC<{ triage?: TriageSummary | null }> = ({ triage }) => {
  if (!triage) return null;
  return (
    <>
      {triage.working && <Pill tone="violet">forge {triage.working}</Pill>}
      {triage.pendingDrafts > 0 && <Pill tone="warn">reply to approve</Pill>}
    </>
  );
};

/* ------------------------------------------------------------------ settings */

interface SourceRow extends DataSourceInput {
  hasHeader: boolean;
}

export const TriageSettingsDialog: React.FC<{
  open: boolean;
  projects: ProjectSummary[];
  onClose: () => void;
}> = ({ open, projects, onClose }) => {
  const [loaded, setLoaded] = React.useState<TriageSettings | null>(null);
  const [project, setProject] = React.useState("");
  const [auto, setAuto] = React.useState(false);
  const [mode, setMode] = React.useState<"approve" | "auto">("approve");
  const [sources, setSources] = React.useState<SourceRow[]>([]);
  const [kube, setKube] = React.useState(false);
  const [kubeContext, setKubeContext] = React.useState("");
  const [autoKinds, setAutoKinds] = React.useState<ReplyKind[]>([]);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setError(null);
    supportApi
      .settings()
      .then((s) => {
        setLoaded(s);
        setProject(s.managerProject ?? "");
        setAuto(s.autoTriage);
        setMode(s.replyMode);
        setAutoKinds(s.autoKinds ?? []);
        setSources(s.sources.map((x) => ({ ...x, header: "" })));
        setKube(s.kubernetes);
        setKubeContext(s.kubeContext ?? "");
      })
      .catch((e: unknown) => setError(errorText(e)));
  }, [open]);

  const patch = (i: number, next: Partial<SourceRow>) =>
    setSources((all) => all.map((s, j) => (j === i ? { ...s, ...next } : s)));

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await supportApi.saveSettings({
        managerProject: project || null,
        autoTriage: auto && !!project,
        replyMode: mode,
        sources: sources.map(({ name, kind, url, header, notes }) => ({ name, kind, url, header, notes })),
        kubernetes: kube,
        kubeContext: kubeContext.trim() || null,
        autoKinds,
      });
      onClose();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl" data-testid="support-triage-settings">
        <DialogHeader>
          <DialogTitle>Support triage</DialogTitle>
          <DialogDescription>
            New tickets go to one project's manager. It investigates read-only, replies only within the rules, and
            leaves everything else to you as an internal note.
          </DialogDescription>
        </DialogHeader>

        {!loaded && !error && <p className="px-6 text-[12.5px] text-muted-foreground">Loading…</p>}
        {loaded && (
          <div className="flex max-h-[62vh] flex-col gap-4 overflow-y-auto px-6 pb-2 text-[12.5px]">
            <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
              Support manager
              <select value={project} onChange={(e) => setProject(e.target.value)} className={inputClass}>
                <option value="">Nobody (hand tickets over yourself)</option>
                {projects.map((p) => (
                  <option key={p.name} value={p.name}>{p.name}</option>
                ))}
              </select>
            </label>

            <label className={cn("flex items-center gap-2 text-foreground", !project && "opacity-50")}>
              <input type="checkbox" checked={auto} disabled={!project} onChange={(e) => setAuto(e.target.checked)} />
              Hand new tickets to it as they arrive
            </label>

            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] text-muted-foreground">Replies to partners</span>
              <Seg<"approve" | "auto">
                label="Replies"
                value={mode}
                onChange={setMode}
                options={[
                  { value: "approve", label: "I approve each one" },
                  { value: "auto", label: "Send when the rules allow" },
                ]}
              />
              <span className="text-[11.5px] text-muted-foreground">
                Either way: nothing before an investigation with evidence, answers only at 90%+, "resolved" only for a
                verified micro fix, and no reply to requests.
              </span>
            </div>

            {mode === "approve" && (
              <div className="flex flex-col gap-1.5">
                <span className="text-[12px] text-muted-foreground">Send these without asking me</span>
                {(loaded.approvals ?? []).map((st) => (
                  <label key={st.kind} className="flex items-center gap-2 text-foreground">
                    <input
                      type="checkbox"
                      checked={autoKinds.includes(st.kind)}
                      onChange={(e) =>
                        setAutoKinds((all) => (e.target.checked ? [...all, st.kind] : all.filter((k) => k !== st.kind)))
                      }
                    />
                    {KIND_LABEL[st.kind]}
                    <span className="text-[11.5px] text-muted-foreground">
                      {st.decided === 0
                        ? "none decided yet"
                        : `${st.unchanged} of ${st.decided} approved as written${st.edited ? `, ${st.edited} edited` : ""}${st.discarded ? `, ${st.discarded} discarded` : ""}`}
                    </span>
                    {st.suggestAuto && !autoKinds.includes(st.kind) && <Pill tone="ok">suggested</Pill>}
                  </label>
                ))}
                <span className="text-[11.5px] text-muted-foreground">
                  Your edits are kept as examples, so the manager writes more like you over time.
                </span>
              </div>
            )}

            <div className="flex flex-col gap-1.5">
              <label className="flex items-center gap-2 text-foreground">
                <input type="checkbox" checked={kube} onChange={(e) => setKube(e.target.checked)} />
                Read the cluster with the server's kubeconfig
              </label>
              <span className="text-[11.5px] text-muted-foreground">
                Get, describe, logs and events; databases and APIs found through Secrets and reached by port-forward.
                Secret values are used, never shown to it, and nothing that changes the cluster runs.
              </span>
              {kube && (
                <input
                  className={cn(inputClass, "font-mono text-[11.5px]")}
                  placeholder="kubeconfig context (blank for the current one)"
                  value={kubeContext}
                  onChange={(e) => setKubeContext(e.target.value)}
                  aria-label="Kubeconfig context"
                />
              )}
            </div>

            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <Database className="size-3.5 text-muted-foreground" aria-hidden="true" />
                <span className="font-medium text-foreground">Data it can read</span>
                <GhostButton
                  size="sm"
                  className="ml-auto"
                  onClick={() => setSources((all) => [...all, { name: "", kind: "mongo", url: "", header: "", notes: "", hasHeader: false }])}
                >
                  <Plus className="size-3.5" aria-hidden="true" /> Add source
                </GhostButton>
              </div>
              <span className="text-[11.5px] text-muted-foreground">
                Use a database user with only the read role, and an API key that can only read. Tendril only ever reads,
                and redacts passwords, hashes and tokens from what comes back.
              </span>
              {sources.length === 0 && <span className="text-[12px] text-muted-foreground">No sources yet.</span>}
              {sources.map((s, i) => (
                <div key={i} className="flex flex-col gap-2 rounded-lg border border-border bg-card p-2.5">
                  <div className="flex gap-2">
                    <input
                      className={cn(inputClass, "w-36 font-mono")}
                      placeholder="name"
                      value={s.name}
                      onChange={(e) => patch(i, { name: e.target.value.replace(/\s/g, "") })}
                      aria-label="Source name"
                    />
                    <select
                      className={cn(inputClass, "w-32")}
                      value={s.kind}
                      onChange={(e) => patch(i, { kind: e.target.value as "mongo" | "http" })}
                      aria-label="Source kind"
                    >
                      <option value="mongo">MongoDB</option>
                      <option value="http">API (GET)</option>
                    </select>
                    <input
                      className={cn(inputClass, "font-mono text-[11.5px]")}
                      placeholder={s.kind === "mongo" ? "mongodb+srv://reader:…@host/db" : "https://api.example.com"}
                      value={s.url}
                      onChange={(e) => patch(i, { url: e.target.value })}
                      aria-label="Source URL"
                    />
                    <button
                      type="button"
                      onClick={() => setSources((all) => all.filter((_, j) => j !== i))}
                      className="shrink-0 rounded-md px-1.5 text-muted-foreground hover:text-destructive"
                      aria-label="Remove source"
                    >
                      <Trash2 className="size-3.5" aria-hidden="true" />
                    </button>
                  </div>
                  {s.kind === "http" && (
                    <input
                      className={cn(inputClass, "font-mono text-[11.5px]")}
                      type="password"
                      placeholder={s.hasHeader ? "Header saved (type to replace, - to remove)" : "Header, e.g. Authorization: Bearer …"}
                      value={s.header ?? ""}
                      onChange={(e) => patch(i, { header: e.target.value })}
                      aria-label="Source header"
                    />
                  )}
                  <input
                    className={inputClass}
                    placeholder="What's in it, for the manager: accounts, challenges, payouts…"
                    value={s.notes}
                    onChange={(e) => patch(i, { notes: e.target.value })}
                    aria-label="Source notes"
                  />
                </div>
              ))}
            </div>
          </div>
        )}
        {error && <p className="px-6 text-[12px] text-destructive">{error}</p>}
        <DialogFooter>
          <GhostButton onClick={onClose}>Cancel</GhostButton>
          <PrimaryButton disabled={busy || !loaded} onClick={() => void save()}>
            {busy ? "Saving…" : "Save"}
          </PrimaryButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

/* ------------------------------------------------------------------ a ticket's case */

/**
 * What the support manager found on a ticket and the replies waiting for approval, above the
 * conversation. Nothing shows for a ticket it never worked.
 */
export const TriageCaseBlock: React.FC<{
  ticketId: string;
  refreshKey: number;
  onChanged: () => void;
  /** The dry run's conversation while one is on, so the panel can show it in place of the hand-off's. */
  onDryRun?: (sessionId: string | null) => void;
}> = ({ ticketId, refreshKey, onChanged, onDryRun }) => {
  const [triage, setTriage] = React.useState<TriageCase | null>(null);
  const [showLookups, setShowLookups] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(() => {
    supportApi.triageCase(ticketId).then(setTriage).catch(() => undefined);
  }, [ticketId]);
  React.useEffect(load, [load, refreshKey]);

  const dry = triage?.dryRunSession ?? null;
  React.useEffect(() => onDryRun?.(dry), [dry, onDryRun]);
  const dryRun = async (start: boolean) => {
    setError(null);
    try {
      if (start) await supportApi.startDryRun(ticketId);
      else await supportApi.endDryRun(ticketId);
      load();
      onChanged();
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (!triage) return null;
  const inv = triage.investigation;
  const pending = triage.drafts.filter((d) => d.state === "pending");
  if (!inv && pending.length === 0 && triage.lookups.length === 0 && !triage.triagedAt) {
    return (
      <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <GhostButton size="sm" onClick={() => void dryRun(true)} title="Work this ticket the way triage would, without posting anything">
          <FlaskConical className="size-3.5" aria-hidden="true" /> Dry run triage
        </GhostButton>
        {error && <span className="text-destructive">{error}</span>}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2" data-testid="support-triage-case">
      <div className="flex flex-col gap-1.5 rounded-lg border border-border bg-card px-3 py-2">
        <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
          <ShieldCheck className="size-3.5 text-primary" aria-hidden="true" />
          <span className="font-medium text-foreground">Triage</span>
          {dry && <Pill tone="info">dry run · nothing posted</Pill>}
          {inv ? (
            <>
              <Pill tone="mute">{CATEGORY_LABEL[inv.category]}</Pill>
              <Pill tone={SEVERITY_TONE[inv.severity]}>{inv.severity}</Pill>
              <span>{inv.confidence}% sure</span>
              {inv.duplicateOf && <span>· same as {inv.duplicateOf.slice(0, 8)}</span>}
            </>
          ) : null}
          {triage.working ? (
            <span className="inline-flex items-center gap-1 text-violet">
              <span className="size-1.5 animate-pulse rounded-full bg-violet" aria-hidden="true" />
              Forge is {triage.working} this
            </span>
          ) : (
            triage.handedOver && <span>· left to you</span>
          )}
          <span className="ml-auto" />
          <button
            type="button"
            className="inline-flex items-center gap-1 hover:text-foreground"
            onClick={() => void dryRun(!dry)}
            title={dry ? "Throw the dry run away" : "Work this ticket again as a dry run, without posting anything"}
          >
            <FlaskConical className="size-3" aria-hidden="true" />
            {dry ? "End dry run" : "Dry run"}
          </button>
          {triage.lookups.length > 0 && (
            <button
              type="button"
              className="inline-flex items-center gap-1 hover:text-foreground"
              onClick={() => setShowLookups((v) => !v)}
            >
              <Search className="size-3" aria-hidden="true" />
              {triage.lookups.length} lookup{triage.lookups.length === 1 ? "" : "s"}
            </button>
          )}
        </div>
        {inv && <p className="m-0 whitespace-pre-wrap text-[12px] text-foreground">{inv.findings}</p>}
        {inv && inv.evidence.length > 0 && (
          <p className="m-0 truncate font-mono text-[10.5px] text-muted-foreground" title={inv.evidence.join("\n")}>
            {inv.evidence.join(" · ")}
          </p>
        )}
        {showLookups && (
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 font-mono text-[10.5px] text-muted-foreground">
            {triage.lookups.map((l, i) => (
              <li key={i} className="truncate" title={l.request}>
                {l.source}: {l.request} → {l.result}
              </li>
            ))}
          </ul>
        )}
      </div>
      {triage.dryNotes?.map((n, i) => (
        <div key={i} className="rounded-lg border border-warning/30 bg-warning/8 px-3 py-2 text-[12px] text-foreground">
          <span className="mb-0.5 block text-[10.5px] text-warning">internal note it would have left</span>
          {n}
        </div>
      ))}
      {pending.map((d) => (
        <DraftCard
          key={d.id}
          ticketId={ticketId}
          dry={!!dry}
          draft={d}
          onDone={() => {
            load();
            onChanged();
          }}
          onError={setError}
        />
      ))}
      {error && <span className="text-[12px] text-destructive">{error}</span>}
    </div>
  );
};

const DraftCard: React.FC<{
  ticketId: string;
  draft: TriageCase["drafts"][number];
  /** A dry run's draft is only shown: it can be discarded, never sent. */
  dry?: boolean;
  onDone: () => void;
  onError: (e: string | null) => void;
}> = ({ ticketId, draft, dry, onDone, onError }) => {
  const [text, setText] = React.useState(draft.content);
  const [busy, setBusy] = React.useState(false);
  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    onError(null);
    try {
      await fn();
      onDone();
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-col gap-2 rounded-lg border border-warning/40 bg-warning/6 px-3 py-2" data-testid="support-draft">
      <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="font-medium text-foreground">{dry ? "Reply it would have sent" : "Reply waiting for you"}</span>
        <Pill tone="warn">{KIND_LABEL[draft.kind]}</Pill>
        {draft.close && <span>· closes the ticket</span>}
        {draft.fixPr && (
          <button
            type="button"
            className="ml-auto truncate font-mono text-info hover:underline"
            onClick={() => void openUrl(draft.fixPr!)}
          >
            {draft.fixPr.replace(/^https:\/\/github\.com\//, "")}
          </button>
        )}
      </div>
      <textarea
        className="min-h-[64px] w-full resize-y rounded-md border border-input bg-muted px-2.5 py-2 text-[12.5px] text-foreground outline-none focus:border-primary/60"
        value={text}
        onChange={(e) => setText(e.target.value)}
        aria-label="Draft reply"
      />
      {draft.test && <span className="text-[11px] text-muted-foreground">Test: {draft.test}</span>}
      <div className="flex items-center gap-2">
        <GhostButton size="sm" disabled={busy} onClick={() => void run(() => supportApi.discardDraft(ticketId, draft.id))}>
          <X className="size-3.5" aria-hidden="true" /> Discard
        </GhostButton>
        {!dry && <PrimaryButton
          size="sm"
          className="ml-auto"
          disabled={busy || !text.trim()}
          onClick={() =>
            void run(() => supportApi.approveDraft(ticketId, draft.id, text.trim() === draft.content ? undefined : text.trim()))
          }
        >
          <Check className="size-3.5" aria-hidden="true" />
          {busy ? "Sending…" : draft.close ? "Approve, send and close" : "Approve and send"}
        </PrimaryButton>}
      </div>
    </div>
  );
};
