import React from "react";
import { Button, Input, Label } from "@ivy-interactive/components/ui";
import type { TelegramStatus } from "../../types/projectAssets";
import { bridge } from "../../api/bridge";
import { SettingsSection } from "./fields";

const EMPTY: TelegramStatus = { configured: false, botUsername: "", paired: false };

/**
 * The Telegram bot the operator talks to their managers through: add the bot, then pair one Telegram
 * account with it. The bot answers that account alone, so pairing is what makes it private; until it
 * is done the bot answers nobody.
 */
export const TelegramSection: React.FC = () => {
  const [status, setStatus] = React.useState<TelegramStatus>(EMPTY);
  const [token, setToken] = React.useState("");
  const [busy, setBusy] = React.useState<string | null>(null);
  const [note, setNote] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(() => bridge.telegram("status").then(setStatus).catch(() => undefined), []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  // While a code is waiting to be sent to the bot, watch for the pairing to land.
  React.useEffect(() => {
    if (!status.configured || status.paired) return;
    const timer = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(timer);
  }, [status.configured, status.paired, refresh]);

  const run = async (action: "set" | "unpair" | "test" | "remove", done?: string) => {
    setBusy(action);
    setError(null);
    setNote(null);
    try {
      const next = await bridge.telegram(action, action === "set" ? token.trim() : undefined);
      if (action === "test") await refresh();
      else setStatus(next);
      if (action === "set") setToken("");
      if (done) setNote(done);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const botLink = status.botUsername ? `https://t.me/${status.botUsername}` : null;
  const startCommand = `/start ${status.pairCode ?? ""}`;

  return (
    <SettingsSection
      title="Telegram"
      hint="Talk to your managers from your phone. Send /manager and a project name to start a conversation with that manager, /end to stop; a manager that needs you messages you there, and you reply to answer it. The bot answers one Telegram account only: yours."
      testId="telegram-card"
    >
      <div className="space-y-4">
        {!status.configured && (
          <div className="space-y-3">
            <ol className="list-decimal space-y-1 pl-5 text-[12.5px] text-muted-foreground">
              <li>
                In Telegram, open <span className="font-medium text-foreground">@BotFather</span>, send{" "}
                <code className="font-mono text-foreground">/newbot</code> and follow it.
              </li>
              <li>Paste the token it gives you here.</li>
            </ol>
            <div className="space-y-1">
              <Label htmlFor="telegram-token" className="text-xs font-medium text-foreground">
                Bot token
              </Label>
              <div className="flex gap-2">
                <Input
                  id="telegram-token"
                  type="password"
                  autoComplete="off"
                  value={token}
                  placeholder="123456789:AA…"
                  onChange={(e) => setToken(e.target.value)}
                  data-testid="telegram-token"
                />
                <Button type="button" disabled={!token.trim() || busy !== null} onClick={() => void run("set")} data-testid="telegram-save">
                  {busy === "set" ? "Checking…" : "Add bot"}
                </Button>
              </div>
            </div>
          </div>
        )}

        {status.configured && !status.paired && (
          <div className="space-y-3 rounded-xl border border-warning/40 bg-warning/10 p-3.5" data-testid="telegram-pairing">
            <div className="text-sm font-medium text-foreground">
              Pair your Telegram account with @{status.botUsername}
            </div>
            <p className="text-[12.5px] text-muted-foreground">
              On your phone, open{" "}
              {botLink ? (
                <a className="font-medium text-foreground underline" href={botLink} target="_blank" rel="noreferrer">
                  t.me/{status.botUsername}
                </a>
              ) : (
                "the bot"
              )}{" "}
              and send it this message. The first account to send the code becomes the only one the bot answers, so do
              not share it.
            </p>
            <div className="flex items-center gap-2">
              <code className="rounded-lg border border-border bg-card px-3 py-2 font-mono text-[15px] tracking-wider text-foreground" data-testid="telegram-code">
                {startCommand}
              </code>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void navigator.clipboard?.writeText(startCommand).then(() => setNote("Copied."))}
              >
                Copy
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">Waiting for your message… this page updates by itself.</p>
          </div>
        )}

        {status.paired && (
          <div className="space-y-2 rounded-xl border border-border bg-card p-3.5 text-[12.5px]" data-testid="telegram-paired">
            <div className="text-foreground">
              <span className="font-medium">@{status.botUsername}</span> answers only{" "}
              <span className="font-medium">{status.pairedWith ?? "your account"}</span>.
            </div>
            <div className="text-muted-foreground">
              {status.talkingTo ? `Right now it is talking to ${status.talkingTo}'s manager.` : "Not in a conversation with a manager right now."}
            </div>
          </div>
        )}

        {status.configured && (
          <div className="flex flex-wrap gap-2">
            {status.paired && (
              <Button type="button" variant="outline" size="sm" disabled={busy !== null} onClick={() => void run("test", "Sent. Check Telegram.")} data-testid="telegram-test">
                Send a test message
              </Button>
            )}
            {status.paired && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={busy !== null}
                onClick={() => void run("unpair", "Unpaired. Send the new code from the account you want to use.")}
                data-testid="telegram-unpair"
              >
                Unpair this account
              </Button>
            )}
            <Button type="button" variant="ghost" size="sm" disabled={busy !== null} onClick={() => void run("remove", "Bot removed.")} data-testid="telegram-remove">
              Remove bot
            </Button>
          </div>
        )}

        {error && <p className="text-xs text-destructive">{error}</p>}
        {note && <p className="text-xs text-muted-foreground">{note}</p>}
      </div>
    </SettingsSection>
  );
};
