import React from "react";
import { Button } from "@ivy-interactive/components/ui";
import { claudeAccountsApi, type ClaudeAccount } from "../../api/agentsApi";
import { openAgentSignIn, useAgentSignIn } from "../../state/agentSignIn";

const until = (iso?: string | null) =>
  iso ? new Date(iso).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : null;

/**
 * The two Claude subscriptions: main, and a fallback that takes over while main is rate limited and
 * hands back once it resets. Each keeps its own login, so neither switch needs a new sign-in.
 */
export const ClaudeAccountsCard: React.FC = () => {
  const [accounts, setAccounts] = React.useState<ClaudeAccount[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const signingIn = useAgentSignIn();

  const load = React.useCallback(
    () =>
      claudeAccountsApi
        .list()
        .then((list) => {
          setAccounts(list);
          setError(null);
        })
        .catch((err: unknown) => setError(err instanceof Error ? err.message : String(err))),
    [],
  );

  // Re-read when the sign-in dialog closes, and every half minute for the limits.
  React.useEffect(() => {
    if (signingIn) return;
    void load();
    const timer = window.setInterval(() => void load(), 30_000);
    return () => window.clearInterval(timer);
  }, [load, signingIn]);

  const row = (a: ClaudeAccount) => {
    const limited = until(a.limitedUntil);
    const isFallback = a.id === "fallback";
    return (
      <div key={a.id} className="flex items-center gap-3 rounded-lg border border-border bg-card px-3 py-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className="flex items-center gap-2 text-[13px] font-medium">
            {isFallback ? "Fallback account" : "Main account"}
            {a.active && a.email && (
              <span className="rounded-full bg-primary/15 px-1.5 font-mono text-[10px] text-primary">in use</span>
            )}
            {limited && (
              <span className="rounded-full bg-warning/15 px-1.5 font-mono text-[10px] text-warning">
                limited until {limited}
              </span>
            )}
          </div>
          <span className="truncate text-xs text-muted-foreground">
            {a.email ?? (isFallback ? "Not set up. Takes over when the main account hits its limit." : "Not signed in")}
          </span>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => openAgentSignIn(isFallback ? "claude-fallback" : "claude")}
        >
          {a.email ? "Sign in again" : "Sign in"}
        </Button>
        {isFallback && a.email && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void claudeAccountsApi.removeFallback().then(setAccounts).catch(() => undefined)}
          >
            Remove
          </Button>
        )}
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-2" data-testid="claude-accounts">
      <div className="text-[13px] font-medium">Claude subscriptions</div>
      <p className="m-0 text-xs text-muted-foreground">
        Runs use the main account. When it hits a rate or usage limit they move to the fallback, then go
        back to main once its limit resets. Each account keeps its own login.
      </p>
      {error && <p className="m-0 text-xs text-destructive">{error}</p>}
      {accounts?.map(row)}
    </div>
  );
};
