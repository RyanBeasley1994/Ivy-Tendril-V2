import * as React from "react";
import { CheckCircle2, ExternalLink, XCircle } from "lucide-react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Spinner,
} from "@ivy-interactive/components/ui";
import { agentsApi, type AgentLoginView } from "../../api/agentsApi";
import { closeAgentSignIn, useAgentSignIn } from "../../state/agentSignIn";
import { describeBridgeError } from "../../types/api";
import { openUrl } from "../../utils/opener";

const NAMES: Record<string, string> = { claude: "Claude Code", codex: "Codex" };

/**
 * Signs an agent's CLI back in on the daemon's machine, which is usually not this one.
 *
 * The daemon runs the CLI's own headless login and hands back the URL it prints. Claude's page ends
 * on a code to paste back, which this relays; Codex's device flow shows a code to enter on its page
 * and finishes by itself, so this polls until it does.
 */
export const AgentSignInDialog: React.FC = () => {
  const agent = useAgentSignIn();
  const [view, setView] = React.useState<AgentLoginView | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [code, setCode] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [attempt, setAttempt] = React.useState(0);
  const name = agent ? (NAMES[agent] ?? agent) : "";

  React.useEffect(() => {
    if (!agent) return;
    let live = true;
    setView(null);
    setError(null);
    setCode("");
    setBusy(true);
    agentsApi.login
      .start(agent)
      .then((next) => {
        if (!live) return;
        setView(next);
        // Opened straight away: the page is the next step either way.
        if (next.url) void openUrl(next.url);
      })
      .catch((err: unknown) => live && setError(describeBridgeError(err)))
      .finally(() => live && setBusy(false));
    return () => {
      live = false;
    };
  }, [agent, attempt]);

  // Codex finishes on its own once the code is entered on its page.
  const polling = !!agent && view?.state === "running" && !view.needsCode;
  React.useEffect(() => {
    if (!polling || !agent) return;
    const timer = window.setInterval(() => {
      agentsApi.login
        .status(agent)
        .then((next) => next && setView(next))
        .catch(() => {});
    }, 2000);
    return () => window.clearInterval(timer);
  }, [polling, agent]);

  const close = () => {
    if (agent && view?.state === "running") void agentsApi.login.cancel(agent).catch(() => {});
    setView(null);
    closeAgentSignIn();
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!agent || !code.trim()) return;
    setBusy(true);
    setError(null);
    agentsApi.login
      .submitCode(agent, code.trim())
      .then(setView)
      .catch((err: unknown) => setError(describeBridgeError(err)))
      .finally(() => setBusy(false));
  };

  const done = view?.state === "succeeded";
  const failed = view?.state === "failed";
  // Codex's one-time code is in what it printed; Claude's output is only the URL and the prompt.
  const showOutput = !!view && (failed || (!view.needsCode && view.state === "running"));

  return (
    <Dialog open={!!agent} onOpenChange={(open) => !open && close()}>
      <DialogContent className="max-w-lg" data-testid="agent-sign-in-dialog">
        <DialogHeader>
          <DialogTitle>Sign in to {name}</DialogTitle>
          <DialogDescription>
            Signs the CLI in on the machine running Tendril, so managers and jobs can use it again.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3 px-6 pb-2 text-[13px]">
          {busy && !view && (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Spinner size="sm" /> Starting the sign-in…
            </p>
          )}

          {done && (
            <p className="flex items-center gap-2 text-success" data-testid="agent-sign-in-done">
              <CheckCircle2 className="size-4" aria-hidden /> Signed in. You can retry the message now.
            </p>
          )}

          {view && !done && view.url && (
            <div className="flex flex-col gap-2">
              <p className="m-0 text-muted-foreground">
                {view.needsCode
                  ? "1. Approve access on the sign-in page. 2. Copy the code it shows and paste it here."
                  : "Approve access on the sign-in page and enter the code below. This closes by itself when it's done."}
              </p>
              <div className="flex items-center gap-2">
                <Button type="button" variant="outline" size="sm" onClick={() => void openUrl(view.url!)}>
                  <ExternalLink className="size-3.5" aria-hidden /> Open sign-in page
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => void navigator.clipboard?.writeText(view.url!).catch(() => {})}
                >
                  Copy link
                </Button>
              </div>
            </div>
          )}

          {view?.needsCode && view.state === "running" && (
            <form onSubmit={submit} className="flex items-center gap-2">
              <Input
                autoFocus
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="Paste the code"
                aria-label="Sign-in code"
                className="font-mono"
              />
              <Button type="submit" disabled={busy || !code.trim()}>
                {busy ? <Spinner size="sm" /> : "Sign in"}
              </Button>
            </form>
          )}

          {showOutput && view.output && (
            <pre className="m-0 max-h-48 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-muted p-2.5 font-mono text-[11.5px] text-foreground">
              {view.output}
            </pre>
          )}

          {polling && (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Spinner size="sm" /> Waiting for you to finish on the page…
            </p>
          )}

          {(error || failed) && (
            <p className="flex items-start gap-2 text-destructive">
              <XCircle className="mt-0.5 size-4 shrink-0" aria-hidden />
              {error ?? "Sign-in didn't complete. The code may have expired — try again."}
            </p>
          )}
        </div>

        <DialogFooter>
          {(error || failed) && (
            <Button type="button" variant="outline" onClick={() => setAttempt((n) => n + 1)}>
              Try again
            </Button>
          )}
          <Button type="button" variant={done ? "default" : "ghost"} onClick={close}>
            {done ? "Done" : "Cancel"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
