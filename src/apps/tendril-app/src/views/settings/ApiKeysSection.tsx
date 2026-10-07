import React from "react";
import { Button, Callout, Input, Label, Spinner, Switch } from "@ivy-interactive/components/ui";
import { Check, Copy, KeyRound, Trash2 } from "lucide-react";
import { bridge, type ApiKeyRecord } from "../../api/bridge";
import { useTranslation } from "../../i18n";
import { describeBridgeError } from "../../types/api";
import { isTauri } from "../../utils/tauri";
import { SettingsSection } from "./fields";

/**
 * Keys for the public API (`/api/public/v1`): list projects, read progress, message a manager. A key is
 * shown once, here, when it is made; the daemon keeps only its hash. Revoking stops it at once.
 */
export const ApiKeysSection: React.FC = () => {
  const { t } = useTranslation("settings");
  const [keys, setKeys] = React.useState<ApiKeyRecord[] | null>(null);
  const [name, setName] = React.useState("");
  const [write, setWrite] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [fresh, setFresh] = React.useState<{ key: string; name: string } | null>(null);
  const [copied, setCopied] = React.useState<string | null>(null);

  // In a browser the API is on the address the app was loaded from; the desktop app does not know it.
  const base = `${isTauri() ? "https://<your-forge-address>" : window.location.origin}/api/public/v1`;

  const load = React.useCallback(async () => {
    try {
      setKeys(await bridge.listApiKeys());
    } catch (e) {
      setKeys([]);
      setError(describeBridgeError(e));
    }
  }, []);
  React.useEffect(() => {
    void load();
  }, [load]);

  const copy = async (what: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(what);
      setTimeout(() => setCopied((c) => (c === what ? null : c)), 1800);
    } catch {
      /* The key is on screen to select by hand. */
    }
  };

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const made = await bridge.createApiKey(name.trim(), write);
      setFresh({ key: made.key, name: made.record.name });
      setName("");
      await load();
    } catch (e) {
      setError(describeBridgeError(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (key: ApiKeyRecord) => {
    setError(null);
    try {
      await bridge.revokeApiKey(key.id);
      if (fresh?.name === key.name) setFresh(null);
      await load();
    } catch (e) {
      setError(describeBridgeError(e));
    }
  };

  return (
    <SettingsSection title={t("apiKeys.title")} hint={t("apiKeys.hint", { base })} testId="api-keys-card">
      <div className="space-y-4">
        {error && <p className="text-xs text-destructive">{error}</p>}

        {fresh && (
          <Callout.Success data-testid="api-key-fresh">
            <div className="space-y-2">
              <p className="text-sm font-medium">{t("apiKeys.created", { name: fresh.name })}</p>
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 select-all break-all rounded-md bg-background/60 px-2.5 py-2 font-mono text-[12px]">
                  {fresh.key}
                </code>
                <Button type="button" variant="outline" size="sm" onClick={() => void copy("key", fresh.key)}>
                  {copied === "key" ? <Check className="size-3.5" aria-hidden /> : <Copy className="size-3.5" aria-hidden />}
                  {copied === "key" ? t("apiKeys.copied") : t("apiKeys.copy")}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">{t("apiKeys.once")}</p>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() =>
                  void copy("curl", `curl -H 'Authorization: Bearer ${fresh.key}' ${base}/projects`)
                }
              >
                {copied === "curl" ? t("apiKeys.copied") : t("apiKeys.copyCurl")}
              </Button>
            </div>
          </Callout.Success>
        )}

        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim()) void create();
          }}
        >
          <div className="min-w-[180px] flex-1 space-y-1.5">
            <Label htmlFor="api-key-name">{t("apiKeys.nameLabel")}</Label>
            <Input
              id="api-key-name"
              value={name}
              placeholder={t("apiKeys.namePlaceholder")}
              maxLength={60}
              onChange={(e) => setName(e.target.value)}
              data-testid="api-key-name"
            />
          </div>
          <div className="flex items-center gap-2 pb-2">
            <Switch id="api-key-write" checked={write} onCheckedChange={setWrite} />
            <Label htmlFor="api-key-write">{t("apiKeys.writeLabel")}</Label>
          </div>
          <Button type="submit" disabled={busy || !name.trim()} data-testid="api-key-create">
            {busy ? <Spinner size="sm" aria-hidden /> : <KeyRound className="size-4" aria-hidden />}
            {t("apiKeys.create")}
          </Button>
        </form>

        <div className="divide-y divide-border rounded-lg border border-border">
          {keys === null && <p className="p-3 text-xs text-muted-foreground">{t("apiKeys.loading")}</p>}
          {keys?.length === 0 && <p className="p-3 text-xs text-muted-foreground">{t("apiKeys.none")}</p>}
          {keys?.map((k) => (
            <div key={k.id} className="flex items-center gap-3 px-3 py-2.5" data-testid="api-key-row">
              <KeyRound className="size-4 shrink-0 text-muted-foreground" aria-hidden />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm">{k.name}</div>
                <div className="font-mono text-[11px] text-muted-foreground">
                  {k.prefix}… · {k.scope === "write" ? t("apiKeys.scopeWrite") : t("apiKeys.scopeRead")} ·{" "}
                  {new Date(k.created).toLocaleDateString()}
                </div>
              </div>
              <Button type="button" variant="ghost" size="sm" onClick={() => void revoke(k)} aria-label={t("apiKeys.revoke")}>
                <Trash2 className="size-4" aria-hidden />
              </Button>
            </div>
          ))}
        </div>
      </div>
    </SettingsSection>
  );
};
