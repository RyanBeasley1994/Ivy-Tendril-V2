import { openUrl } from "./opener";

/** Schemes the operating system should hand to another app rather than the webview loading them. */
const EXTERNAL_SCHEMES = new Set(["http:", "https:", "mailto:", "tel:"]);

/**
 * Whether a link should leave the app for the system browser (or mail client).
 *
 * Same-origin links stay: they are the app's own routes and assets, and in development the app itself
 * is served from a loopback `http` origin, so "is it http" alone would send the app's own pages out.
 * `blob:` and `data:` stay too - they are things the app made, usually a download.
 */
export function isExternalHref(href: string, appOrigin: string): boolean {
  let url: URL;
  try {
    url = new URL(href, appOrigin);
  } catch {
    return false;
  }
  if (!EXTERNAL_SCHEMES.has(url.protocol)) return false;
  return url.protocol === "mailto:" || url.protocol === "tel:" || url.origin !== appOrigin;
}

/**
 * Opens every external link in the system browser instead of the desktop app's webview.
 *
 * Left alone, a plain `<a href>` navigates the webview itself - the whole app is replaced by the page
 * and there is no back button - and `target="_blank"` or `window.open` gets a bare in-app window. So
 * clicks are caught in the capture phase, before any component's own handler, and `window.open` is
 * routed the same way. A loopback URL still goes through `openUrl`'s port forward, so a dev server on a
 * remote daemon opens that server's page rather than this machine's.
 *
 * Links inside an iframe (the Review pane's WebViewer) are the previewed app's own and are not seen
 * here: a capture listener on this document does not receive another document's clicks.
 */
export function installExternalLinkHandler(): void {
  const appOrigin = window.location.origin;

  const onClick = (e: MouseEvent) => {
    // Primary and middle clicks only; a right click is the context menu's.
    if (e.defaultPrevented || (e.button !== 0 && e.button !== 1)) return;
    const anchor = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!anchor || anchor.hasAttribute("download")) return;
    if (!isExternalHref(anchor.href, appOrigin)) return;
    e.preventDefault();
    e.stopPropagation();
    void openUrl(anchor.href);
  };
  document.addEventListener("click", onClick, true);
  // A middle click fires `auxclick`, not `click`.
  document.addEventListener("auxclick", onClick, true);

  const nativeOpen = window.open.bind(window);
  window.open = (url?: string | URL, target?: string, features?: string) => {
    const href = url === undefined ? "" : String(url);
    if (href && isExternalHref(href, appOrigin)) {
      void openUrl(new URL(href, appOrigin).href);
      return null;
    }
    return nativeOpen(url, target, features);
  };
}
