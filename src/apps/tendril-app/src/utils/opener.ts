import * as tauriOpener from "@tauri-apps/plugin-opener";
import { isTauri } from "./tauri";

/**
 * `@tauri-apps/plugin-opener`, with a browser fallback.
 *
 * A URL opens in a new tab. A local path cannot: it names a file on the daemon's machine, which a
 * browser has no way to open or reveal, so those reject with the same `UNSUPPORTED_IN_BROWSER`
 * the HTTP client uses, and the caller's existing error handling reports it.
 */
export async function openUrl(url: string | URL): Promise<void> {
  if (isTauri()) return tauriOpener.openUrl(url);
  window.open(String(url), "_blank", "noopener,noreferrer");
}

export async function openPath(path: string): Promise<void> {
  if (isTauri()) return tauriOpener.openPath(path);
  throw {
    code: "UNSUPPORTED_IN_BROWSER",
    message: `'${path}' can only be opened in the desktop app`,
  };
}

export async function revealItemInDir(path: string): Promise<void> {
  if (isTauri()) return tauriOpener.revealItemInDir(path);
  throw {
    code: "UNSUPPORTED_IN_BROWSER",
    message: `'${path}' can only be shown in the desktop app`,
  };
}
