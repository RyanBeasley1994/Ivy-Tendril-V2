import React from "react";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "@ivy-interactive/components/theme";
import { App } from "./App";
import { installBrowserTransport } from "./api/httpClient";
import { ProxyOriginProvider } from "./api/proxyOrigin";
import { initI18n } from "./i18n";
import { startupLanguage, syncDocumentLanguage } from "./state/language";
import { applyAppZoom } from "./state/zoom";
import { isTauri } from "./utils/tauri";
import { loopbackPort } from "./utils/loopbackUrl";
import { openUrl } from "./utils/opener";
import { registerPwa, trackVisualViewport } from "./pwa/register";
import "./index.css";

applyAppZoom();

// Outside the desktop app there is no Tauri IPC: the page is served by the authenticating proxy in
// front of the daemon, and every call goes to the daemon's HTTP routes through it instead.
if (!isTauri()) {
  installBrowserTransport();
}

// The installable web app: a service worker (installability, offline page, push) and a viewport height
// that follows the on-screen keyboard. Service workers need a secure context (HTTPS, or localhost), so on
// a plain-http address this quietly does nothing and the app still works as a normal page.
if (!isTauri()) {
  registerPwa();
  trackVisualViewport();
}

// The macOS window has no title bar (`titleBarStyle: Overlay`): the traffic lights float over the
// sidebar, so the layout leaves them room. Other platforms keep their native frame.
if (isTauri() && /Mac/.test(navigator.platform)) {
  document.documentElement.classList.add("tauri-macos");
}

// A loopback link anywhere in the app (a chat answer saying "serving on http://localhost:5173", a plan,
// a log) means the *server's* loopback when connected to a remote one: open it through a port forward
// rather than letting the webview navigate to this machine's.
if (isTauri()) {
  document.addEventListener(
    "click",
    (e) => {
      const anchor = (e.target as HTMLElement | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!anchor || loopbackPort(anchor.href) === null) return;
      e.preventDefault();
      e.stopPropagation();
      void openUrl(anchor.href);
    },
    true,
  );
}

const rootElement = document.getElementById("root");
if (rootElement) {
  // The catalogs load before the first render, so the shell never paints a translation key or a frame
  // of the wrong language: there is no Suspense boundary above `App` to show while they arrive. The
  // language is the one the last session applied, or the operating system's; `App` then applies what
  // config.yaml says. `initI18n` never rejects, so a catalog that fails to load costs the translation,
  // not the app.
  void initI18n(startupLanguage()).then(() => {
    syncDocumentLanguage();
    ReactDOM.createRoot(rootElement).render(
      <React.StrictMode>
        <ThemeProvider defaultTheme="dark" storageKey="tendril-theme">
          {/* Answers, once for the whole shell, where the WebViewer's proxy lives. See
              `api/proxyOrigin` for why this cannot be left to the call sites. */}
          <ProxyOriginProvider>
            <App />
          </ProxyOriginProvider>
        </ThemeProvider>
      </React.StrictMode>,
    );
  });
}
