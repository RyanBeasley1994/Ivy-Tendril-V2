/** Registers the service worker. Skipped in the desktop app and wherever the browser offers none. */
export function registerPwa(): void {
  if (!("serviceWorker" in navigator) || !window.isSecureContext) return;
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/pwa-sw.js", { scope: "/" }).catch(() => {
      /* An unreachable or login-gated worker script only costs installability. */
    });
  });
}

/**
 * Publishes the visible height as `--app-height`. `100dvh` ignores the on-screen keyboard on iOS, which
 * leaves the composer hidden behind it; the visual viewport shrinks with the keyboard, so layouts that
 * pin to the bottom (the chat composer, the tab bar) follow it.
 */
export function trackVisualViewport(): void {
  const vv = window.visualViewport;
  const root = document.documentElement;
  const apply = () => {
    root.style.setProperty("--app-height", `${Math.round(vv ? vv.height : window.innerHeight)}px`);
    // Safari scrolls the page up when the keyboard opens; undo it so fixed bars stay put.
    if (vv && vv.offsetTop > 0) window.scrollTo(0, 0);
  };
  apply();
  vv?.addEventListener("resize", apply);
  vv?.addEventListener("scroll", apply);
  window.addEventListener("orientationchange", apply);
}

/** True when running as an installed app rather than in a browser tab. */
export function isStandalone(): boolean {
  return (
    window.matchMedia("(display-mode: standalone)").matches ||
    (navigator as unknown as { standalone?: boolean }).standalone === true
  );
}
