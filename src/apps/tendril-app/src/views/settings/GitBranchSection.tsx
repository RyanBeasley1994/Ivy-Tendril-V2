import React from "react";
import { GitBranch } from "lucide-react";
import { Button, Label, Switch } from "@ivy-interactive/components/ui";
import type { BranchPreview, GitSettings, TendrilConfig } from "../../types/api";
import { describeBridgeError } from "../../types/api";
import { bridge } from "../../api/bridge";
import { useTranslation } from "../../i18n";
import { notificationsStore } from "../../state/notificationsStore";
import { SaveError, SettingsSection, TextField } from "./fields";

/** The defaults the daemon applies to an absent field (`git/branch_naming.rs`). */
const DEFAULT_PREFIX = "tendril/";
const DEFAULT_TEMPLATE = "{prefix}{folder}";

/** The tokens a template may use, in the order the help lists them. Mirrors `BRANCH_TOKENS`. */
const TOKENS = [
  "prefix",
  "folder",
  "id",
  "title",
  "slug",
  "project",
  "level",
  "date",
  "mission",
  "milestone",
] as const;

const readGit = (config: TendrilConfig | null): GitSettings => {
  const raw = config?.raw?.git;
  if (typeof raw !== "object" || raw === null) return {};
  const git = raw as Record<string, unknown>;
  const str = (key: string) => (typeof git[key] === "string" ? (git[key] as string) : undefined);
  return {
    branchPrefix: str("branchPrefix"),
    branchTemplate: str("branchTemplate"),
    missionBranchTemplate: str("missionBranchTemplate"),
    signCommits: typeof git.signCommits === "boolean" ? git.signCommits : undefined,
  };
};

interface GitForm {
  branchPrefix: string;
  branchTemplate: string;
  missionBranchTemplate: string;
  /** Checked means "leave signing to git config" - the default. */
  signCommits: boolean;
}

/** Blank fields are left out, so the daemon's default applies rather than an empty template. */
const toWire = (form: GitForm): GitSettings => {
  const out: GitSettings = {};
  if (form.branchPrefix.trim()) out.branchPrefix = form.branchPrefix.trim();
  if (form.branchTemplate.trim()) out.branchTemplate = form.branchTemplate.trim();
  if (form.missionBranchTemplate.trim())
    out.missionBranchTemplate = form.missionBranchTemplate.trim();
  // Only the non-default value is written, so an untouched install keeps no `signCommits` key.
  if (!form.signCommits) out.signCommits = false;
  return out;
};

/**
 * Settings → Git & Branches: how plan and mission branches are named. Each plan's name is fixed the
 * first time it gets a worktree, so a change here applies to new plans only - an in-flight plan keeps
 * the branch it already has, which is what CreatePr and RetryPlan look for. The preview is rendered by
 * the daemon, so it is exactly what a plan would get.
 */
export const GitBranchSection: React.FC<{
  config: TendrilConfig | null;
  onSaveRaw: (key: string, value: unknown) => Promise<void>;
}> = ({ config, onSaveRaw }) => {
  const { t } = useTranslation("settings");
  const saved = React.useMemo(() => readGit(config), [config]);
  const [form, setForm] = React.useState<GitForm>({
    branchPrefix: "",
    branchTemplate: "",
    missionBranchTemplate: "",
    signCommits: true,
  });
  const [preview, setPreview] = React.useState<BranchPreview | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    setForm({
      branchPrefix: saved.branchPrefix ?? "",
      branchTemplate: saved.branchTemplate ?? "",
      missionBranchTemplate: saved.missionBranchTemplate ?? "",
      signCommits: saved.signCommits !== false,
    });
  }, [saved]);

  // Debounced so typing a template is not a request per keystroke.
  React.useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void bridge
        .previewBranchNames(toWire(form))
        .then((p) => {
          if (!cancelled) setPreview(p);
        })
        .catch(() => {
          if (!cancelled) setPreview(null);
        });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [form]);

  const unknown = preview?.unknownTokens ?? [];
  const dirty =
    form.branchPrefix !== (saved.branchPrefix ?? "") ||
    form.branchTemplate !== (saved.branchTemplate ?? "") ||
    form.missionBranchTemplate !== (saved.missionBranchTemplate ?? "") ||
    form.signCommits !== (saved.signCommits !== false);

  const save = async () => {
    if (unknown.length > 0) return;
    setIsSaving(true);
    setError(null);
    try {
      await onSaveRaw("git", toWire(form));
      notificationsStore.notifySuccess(t("git.title"), t("git.saved"));
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setIsSaving(false);
    }
  };

  const set =
    (key: "branchPrefix" | "branchTemplate" | "missionBranchTemplate") => (value: string) =>
      setForm((current) => ({ ...current, [key]: value }));

  return (
    <SettingsSection title={t("git.title")} hint={t("git.hint")} testId="git-settings-card">
      <form
        className="max-w-120 space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <TextField
          id="git-branch-prefix"
          label={t("git.prefixLabel")}
          value={form.branchPrefix}
          placeholder={DEFAULT_PREFIX}
          hint={t("git.prefixHint")}
          onChange={set("branchPrefix")}
        />
        <TextField
          id="git-branch-template"
          label={t("git.templateLabel")}
          value={form.branchTemplate}
          placeholder={DEFAULT_TEMPLATE}
          hint={t("git.templateHint")}
          onChange={set("branchTemplate")}
        />
        <TextField
          id="git-mission-template"
          label={t("git.missionTemplateLabel")}
          value={form.missionBranchTemplate}
          placeholder={DEFAULT_TEMPLATE}
          hint={t("git.missionTemplateHint")}
          onChange={set("missionBranchTemplate")}
        />

        <div
          className="space-y-1.5 rounded-lg border border-border p-3"
          data-testid="git-branch-preview"
        >
          <p className="text-xs font-medium text-foreground">{t("git.previewTitle")}</p>
          {preview ? (
            (["plan", "milestone", "mission"] as const).map((kind) => (
              <div key={kind} className="flex items-center gap-2 text-xs">
                <span className="w-20 shrink-0 text-muted-foreground">
                  {t(`git.preview.${kind}`)}
                </span>
                <GitBranch className="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
                <code className="truncate font-mono text-foreground">{preview[kind]}</code>
              </div>
            ))
          ) : (
            <p className="text-xs text-muted-foreground">{t("git.previewUnavailable")}</p>
          )}
          {unknown.length > 0 && (
            <p className="text-xs text-destructive">
              {t("git.unknownTokens", { tokens: unknown.map((u) => `{${u}}`).join(", ") })}
            </p>
          )}
        </div>

        <div className="space-y-1">
          <p className="text-xs font-medium text-foreground">{t("git.tokensTitle")}</p>
          <ul className="m-0 grid list-none gap-x-4 gap-y-1 p-0 sm:grid-cols-2">
            {TOKENS.map((token) => (
              <li key={token} className="text-xs text-muted-foreground">
                <code className="font-mono text-foreground">{`{${token}}`}</code>{" "}
                {t(`git.tokens.${token}`)}
              </li>
            ))}
          </ul>
        </div>

        <p className="text-xs text-muted-foreground">{t("git.stableNote")}</p>

        <div className="space-y-1">
          <div className="flex items-center gap-3">
            <Switch
              id="git-sign-commits"
              checked={form.signCommits}
              onCheckedChange={(checked) =>
                setForm((current) => ({ ...current, signCommits: checked }))
              }
            />
            <Label htmlFor="git-sign-commits" className="text-xs font-medium text-foreground">
              {t("git.signLabel")}
            </Label>
          </div>
          <p className="text-xs text-muted-foreground">{t("git.signHint")}</p>
        </div>
        <SaveError message={error} />
        <Button type="submit" disabled={!dirty || isSaving || unknown.length > 0}>
          {t("git.save")}
        </Button>
      </form>
    </SettingsSection>
  );
};
