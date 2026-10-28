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
import { installExternalLinkHandler } from "./utils/externalLinks";
import { registerPwa, trackVisualViewport } from "./pwa/register";
// Bundled rather than fetched: the desktop app's CSP only loads fonts it ships (`font-src 'self'`).
import "@fontsource-variable/onest";
import "@fontsource-variable/geist-mono";
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

// Every external link opens in the system browser rather than navigating the app's own webview. A
// loopback one (a chat answer saying "serving on http://localhost:5173") is port-forwarded first when
// connected to a remote server, so it opens the *server's* dev server. See `utils/externalLinks`.
if (isTauri()) {
  installExternalLinkHandler();
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
