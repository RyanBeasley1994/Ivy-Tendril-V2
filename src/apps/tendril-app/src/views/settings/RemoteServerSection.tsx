import React from "react";
import { Button, Callout, Input, Label, Spinner } from "@ivy-interactive/components/ui";
import { Server } from "lucide-react";
import { remoteApi, type RemoteApi, type RemoteConnection } from "../../api/remoteApi";
import { useTranslation } from "../../i18n";
import { describeBridgeError } from "../../types/api";
import { SettingsSection } from "./fields";
import { PortForwardsPanel } from "./PortForwardsPanel";

export interface RemoteServerSectionProps {
  /** Injectable for tests; the Tauri-backed API otherwise. */
  api?: RemoteApi;
}

/**
 * Points the desktop app at a Tendril server running somewhere else, such as a VPS, by address and
 * password. The server has to have session protection on (a password set), since that is the only
 * credential a remote caller can hold.
 *
 * Connecting logs in first and only then saves, so a wrong password or a typo in the address is
 * reported here and leaves the current connection alone. Success restarts the app onto the new
 * server; see `service::remote` in src-tauri for why it is a restart.
 */
export const RemoteServerSection: React.FC<RemoteServerSectionProps> = ({ api = remoteApi }) => {
  const { t } = useTranslation("settings");
  const [connection, setConnection] = React.useState<RemoteConnection | null>(null);
  const [url, setUrl] = React.useState("");
  const [username, setUsername] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [restarting, setRestarting] = React.useState(false);

  React.useEffect(() => {
    let live = true;
    api
      .getConnection()
      .then((c) => live && setConnection(c))
      .catch(() => live && setConnection(null));
    return () => {
      live = false;
    };
  }, [api]);

  const connect = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.connect(url.trim(), username.trim(), password);
      setRestarting(true);
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setBusy(false);
    }
  };

  const disconnect = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.disconnect();
      setRestarting(true);
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsSection
      title={t("remoteServer.title")}
      hint={t("remoteServer.hint")}
      testId="remote-server-section"
    >
      <div className="space-y-4">
        {restarting && (
          <Callout.Info data-testid="remote-server-restarting">
            {t("remoteServer.restarting")}
          </Callout.Info>
        )}
        {error && (
          <p className="text-xs text-destructive" data-testid="remote-server-error">
            {error}
          </p>
        )}

        {connection?.active ? (
          <div className="space-y-3" data-testid="remote-server-connected">
            <div className="flex items-center gap-2 rounded-selector bg-muted/50 p-3 text-sm">
              <Server className="size-4 shrink-0 text-primary" aria-hidden />
              <div className="min-w-0 flex-1">
                <p className="truncate font-mono text-xs text-foreground">{connection.url}</p>
                <p className="text-xs text-muted-foreground">
                  {connection.authenticated
                    ? t("remoteServer.connectedAs", {
                        username: connection.username || t("remoteServer.defaultUser"),
                      })
                    : t("remoteServer.notAuthenticated", { error: connection.error ?? "" })}
                </p>
              </div>
            </div>
            <Button
              type="button"
              variant="outline"
              disabled={busy || restarting}
              onClick={() => void disconnect()}
              data-testid="remote-server-disconnect"
            >
              {busy && <Spinner size="sm" aria-hidden />}
              {t("remoteServer.disconnect")}
            </Button>
            <PortForwardsPanel />
          </div>
        ) : (
          <form
            className="space-y-3"
            onSubmit={(e) => {
              e.preventDefault();
              void connect();
            }}
          >
            <div className="space-y-1.5">
              <Label htmlFor="remote-server-url">{t("remoteServer.urlLabel")}</Label>
              <Input
                id="remote-server-url"
                value={url}
                placeholder={t("remoteServer.urlPlaceholder")}
                autoComplete="url"
                onChange={(e) => setUrl(e.target.value)}
                data-testid="remote-server-url"
              />
              <p className="text-xs text-muted-foreground">{t("remoteServer.urlHint")}</p>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="remote-server-username">{t("remoteServer.usernameLabel")}</Label>
              <Input
                id="remote-server-username"
                value={username}
                placeholder={t("remoteServer.defaultUser")}
                autoComplete="username"
                onChange={(e) => setUsername(e.target.value)}
                data-testid="remote-server-username"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="remote-server-password">{t("remoteServer.passwordLabel")}</Label>
              <Input
                id="remote-server-password"
                type="password"
                value={password}
                autoComplete="current-password"
                onChange={(e) => setPassword(e.target.value)}
                data-testid="remote-server-password"
              />
            </div>
            <Button
              type="submit"
              disabled={busy || restarting || url.trim() === "" || password === ""}
              data-testid="remote-server-connect"
            >
              {busy && <Spinner size="sm" aria-hidden />}
              {t("remoteServer.connect")}
            </Button>
          </form>
        )}
      </div>
    </SettingsSection>
  );
};
