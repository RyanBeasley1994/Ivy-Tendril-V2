import React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, GitFork } from "lucide-react";
import { AUTHOR, PRODUCT_NAME, UPSTREAM } from "../branding";
import { isTauri } from "../utils/tauri";

export interface AboutViewProps {
  /** The running version, when the daemon has answered. */
  version?: string | null;
}

const open = (url: string) => {
  if (isTauri()) void openUrl(url);
  else window.open(url, "_blank", "noopener");
};

/**
 * What this app is and where it came from. The credit and the license notice are required, not
 * decoration: the upstream code is used under its license, which asks that its notices travel with it.
 */
export const AboutView: React.FC<AboutViewProps> = ({ version }) => (
  <div className="mx-auto max-w-2xl space-y-8 py-6" data-testid="about-view">
    <header className="space-y-1">
      <h1 className="text-2xl font-semibold tracking-tight text-foreground">{PRODUCT_NAME}</h1>
      <p className="text-sm text-muted-foreground">
        {version ? `Version ${version} · ` : ""}Built and maintained by {AUTHOR}
      </p>
    </header>

    <section className="space-y-3 rounded-box border border-border bg-card p-5">
      <div className="flex items-center gap-2 text-sm font-medium text-foreground">
        <GitFork className="size-4 text-primary" aria-hidden />
        Credit
      </div>
      <p className="text-sm leading-relaxed text-muted-foreground">
        {PRODUCT_NAME} is a heavily edited and extended fork of{" "}
        <span className="text-foreground">{UPSTREAM.name}</span>, created by{" "}
        <span className="text-foreground">{UPSTREAM.author}</span>. Their work is the foundation
        this is built on - thank you.
      </p>
      <button
        type="button"
        onClick={() => open(UPSTREAM.url)}
        className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
      >
        {UPSTREAM.name} on GitHub
        <ExternalLink className="size-3.5" aria-hidden />
      </button>
    </section>

    <section className="space-y-2">
      <h2 className="text-sm font-medium text-foreground">License</h2>
      <p className="text-sm leading-relaxed text-muted-foreground">
        Portions of this software are {UPSTREAM.copyright}, used and modified under the{" "}
        {UPSTREAM.license}. The full license text ships with the source in <code>LICENSE</code>.
      </p>
    </section>
  </div>
);
