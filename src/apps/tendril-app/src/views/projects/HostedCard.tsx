import React from "react";
import { Check, Copy, ExternalLink, Eye, EyeOff, Globe, Pencil, Trash2 } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { ProjectLink, ProjectLinkInput } from "../../types/projectAssets";
import { bridge } from "../../api/bridge";
import { openUrl } from "../../utils/opener";
import { formatAge } from "../../utils/commandCenter";
import { Card, GhostButton, IconButton, Pill, PrimaryButton } from "../../components/page/kit";

/**
 * What the project has running: dev servers, preview deploys, admin panels, and the test logins that
 * go with them. Agents record these with `tendril link set` so they are not lost in the chat; the
 * card stays out of the way until there is at least one.
 */
export const HostedCard: React.FC<{ project: string }> = ({ project }) => {
  const [links, setLinks] = React.useState<ProjectLink[]>([]);
  const [editing, setEditing] = React.useState<string | null>(null);

  const load = React.useCallback(
    () =>
      bridge
        .listProjectLinks(project)
        .then(setLinks)
        .catch(() => undefined),
    [project],
  );

  React.useEffect(() => {
    setLinks([]);
    void load();
    // An agent records a link mid-run, so keep looking while the page is open.
    const timer = window.setInterval(() => void load(), 8_000);
    return () => window.clearInterval(timer);
  }, [load]);

  if (links.length === 0) return null;

  const save = async (link: ProjectLinkInput) => {
    await bridge.saveProjectLink(project, link);
    setEditing(null);
    await load();
  };
  const remove = async (slug: string) => {
    await bridge.deleteProjectLink(project, slug);
    await load();
  };

  return (
    <Card
      title="Hosted"
      meta={<Pill>{links.length}</Pill>}
      actions={<Globe className="size-3.5 text-muted-foreground" aria-hidden="true" />}
      bodyClassName="divide-y divide-border/60"
      testId="hosted-card"
    >
      {links.map((link) =>
        editing === link.slug ? (
          <LinkForm key={link.slug} link={link} onSave={save} onCancel={() => setEditing(null)} />
        ) : (
          <LinkRow key={link.slug} link={link} onEdit={() => setEditing(link.slug)} onRemove={() => remove(link.slug)} />
        ),
      )}
    </Card>
  );
};

const LinkRow: React.FC<{ link: ProjectLink; onEdit: () => void; onRemove: () => Promise<void> }> = ({
  link,
  onEdit,
  onRemove,
}) => {
  const [confirming, setConfirming] = React.useState(false);
  React.useEffect(() => {
    if (!confirming) return;
    const timer = window.setTimeout(() => setConfirming(false), 3_000);
    return () => window.clearTimeout(timer);
  }, [confirming]);

  const age = formatAge(Math.max(0, Date.now() - new Date(link.updated).getTime()));
  return (
    <div className="group flex flex-col gap-1.5 px-4 py-3" data-testid={`hosted-link-${link.slug}`}>
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground">{link.label}</span>
        <span className="hidden shrink-0 font-mono text-[10.5px] text-muted-foreground sm:inline">
          {link.source && link.source !== "user" ? `${link.source} · ` : ""}
          {age} ago
        </span>
        <div className="flex shrink-0 items-center">
          <IconButton label="Edit" className="size-7" onClick={onEdit}>
            <Pencil className="size-3.5" aria-hidden="true" />
          </IconButton>
          {confirming ? (
            <GhostButton size="sm" className="text-destructive" onClick={() => void onRemove()}>
              Remove?
            </GhostButton>
          ) : (
            <IconButton label="Remove" className="size-7" onClick={() => setConfirming(true)}>
              <Trash2 className="size-3.5" aria-hidden="true" />
            </IconButton>
          )}
        </div>
      </div>
      <button
        type="button"
        onClick={() => void openUrl(link.url)}
        title={`Open ${link.url}`}
        className="flex min-w-0 items-center gap-1.5 self-start font-mono text-[12px] text-primary hover:underline"
      >
        <span className="truncate">{link.url}</span>
        <ExternalLink className="size-3 shrink-0" aria-hidden="true" />
      </button>
      {(link.username || link.password) && (
        <div className="flex flex-wrap gap-x-4 gap-y-1">
          {link.username && <Secret label="user" value={link.username} />}
          {link.password && <Secret label="pass" value={link.password} hidden />}
        </div>
      )}
      {link.notes && <p className="m-0 text-[12px] text-muted-foreground">{link.notes}</p>}
    </div>
  );
};

/** A login field: copyable, and masked until asked for when it is a password. */
const Secret: React.FC<{ label: string; value: string; hidden?: boolean }> = ({ label, value, hidden }) => {
  const [shown, setShown] = React.useState(!hidden);
  const [copied, setCopied] = React.useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_500);
    } catch {
      // A webview without clipboard access: reveal it so it can be selected by hand.
      setShown(true);
    }
  };
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 font-mono text-[11.5px]">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 select-all truncate text-foreground">{shown ? value : "••••••••"}</span>
      {hidden && (
        <button
          type="button"
          onClick={() => setShown((s) => !s)}
          aria-label={shown ? "Hide password" : "Show password"}
          className="text-muted-foreground hover:text-foreground"
        >
          {shown ? <EyeOff className="size-3" aria-hidden="true" /> : <Eye className="size-3" aria-hidden="true" />}
        </button>
      )}
      <button
        type="button"
        onClick={() => void copy()}
        aria-label={`Copy ${label}`}
        className={cn(copied ? "text-success" : "text-muted-foreground hover:text-foreground")}
      >
        {copied ? <Check className="size-3" aria-hidden="true" /> : <Copy className="size-3" aria-hidden="true" />}
      </button>
    </span>
  );
};

const FIELDS: { key: keyof ProjectLinkInput; label: string; mono?: boolean }[] = [
  { key: "label", label: "Label" },
  { key: "url", label: "URL", mono: true },
  { key: "username", label: "Username", mono: true },
  { key: "password", label: "Password", mono: true },
  { key: "notes", label: "Notes" },
];

const LinkForm: React.FC<{
  link: ProjectLink;
  onSave: (link: ProjectLinkInput) => Promise<void>;
  onCancel: () => void;
}> = ({ link, onSave, onCancel }) => {
  const [draft, setDraft] = React.useState<ProjectLinkInput>({
    slug: link.slug,
    label: link.label,
    url: link.url,
    username: link.username,
    password: link.password,
    notes: link.notes,
  });
  const [error, setError] = React.useState<string | null>(null);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      await onSave(draft);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  return (
    <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-2 px-4 py-3">
      {FIELDS.map((f) => (
        <label key={f.key} className="flex items-center gap-3">
          <span className="w-[72px] shrink-0 text-[11.5px] text-muted-foreground">{f.label}</span>
          <input
            value={draft[f.key] ?? ""}
            onChange={(e) => setDraft((d) => ({ ...d, [f.key]: e.target.value }))}
            spellCheck={false}
            className={cn(
              "h-7 min-w-0 flex-1 rounded-md border border-input bg-muted px-2 text-[12.5px] text-foreground outline-none focus:border-primary/60",
              f.mono && "font-mono text-[12px]",
            )}
          />
        </label>
      ))}
      {error && <span className="text-[12px] text-destructive">{error}</span>}
      <div className="flex justify-end gap-2">
        <GhostButton size="sm" onClick={onCancel}>Cancel</GhostButton>
        <PrimaryButton size="sm" type="submit" disabled={!draft.url.trim()}>Save</PrimaryButton>
      </div>
    </form>
  );
};
