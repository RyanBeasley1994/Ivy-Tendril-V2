import React from "react";
import { Button, Input } from "@ivy-interactive/components/ui";
import { ExternalLink, X } from "lucide-react";
import { portForwardsApi, usePortForwards } from "../../api/portForwards";
import { useTranslation } from "../../i18n";
import { describeBridgeError } from "../../types/api";
import { openUrl } from "../../utils/opener";

/**
 * Port forwards to the remote server (`ssh -L` over the Forge connection): any port on the server's
 * loopback, opened here. Dev servers a review action or a chat announces as `localhost:<port>` are
 * forwarded automatically when their link is opened; this is for everything else.
 */
export const PortForwardsPanel: React.FC<{
  /** In a dialog: no divider or heading of its own, the dialog supplies them. */
  bare?: boolean;
}> = ({ bare = false }) => {
  const { t } = useTranslation("settings");
  const forwards = usePortForwards();
  const [port, setPort] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  const add = async () => {
    const n = Number.parseInt(port, 10);
    if (!Number.isFinite(n)) return;
    setBusy(true);
    setError(null);
    try {
      await portForwardsApi.forward(n);
      setPort("");
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={bare ? "space-y-3" : "space-y-2 border-t border-border/70 pt-3"} data-testid="port-forwards">
      {bare ? (
        <p className="m-0 text-xs text-muted-foreground">{t("remoteServer.forwards.hint")}</p>
      ) : (
        <div>
          <p className="m-0 text-sm font-medium text-foreground">{t("remoteServer.forwards.title")}</p>
          <p className="m-0 text-xs text-muted-foreground">{t("remoteServer.forwards.hint")}</p>
        </div>
      )}
      {forwards.length === 0 && bare && (
        <p className="m-0 rounded-selector border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground">
          {t("remoteServer.forwards.none")}
        </p>
      )}
      {forwards.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {forwards.map((f) => (
            <li
              key={f.remotePort}
              className="flex items-center gap-2 rounded-selector bg-muted/40 px-2.5 py-1.5 font-mono text-xs"
            >
              <span className="flex-1 truncate">
                {t("remoteServer.forwards.row", { remote: f.remotePort, local: f.localPort })}
              </span>
              <button
                type="button"
                className="inline-flex items-center gap-1 text-success hover:underline"
                onClick={() => void openUrl(`http://127.0.0.1:${f.localPort}/`)}
              >
                {t("remoteServer.forwards.open")}
                <ExternalLink className="size-3" aria-hidden />
              </button>
              <button
                type="button"
                aria-label={t("remoteServer.forwards.stop", { port: f.remotePort })}
                className="text-muted-foreground hover:text-foreground"
                onClick={() => void portForwardsApi.stop(f.remotePort)}
              >
                <X className="size-3.5" aria-hidden />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <Input
          inputMode="numeric"
          value={port}
          placeholder="5173"
          aria-label={t("remoteServer.forwards.portLabel")}
          className="w-28"
          onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
          data-testid="port-forward-input"
        />
        <Button type="submit" variant="outline" disabled={busy || port === ""}>
          {t("remoteServer.forwards.add")}
        </Button>
      </form>
      {error && <p className="m-0 text-xs text-destructive">{error}</p>}
    </div>
  );
};
