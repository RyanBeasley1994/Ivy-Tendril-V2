import { isTauri } from "../utils/tauri";

/**
 * The desktop window's page zoom. The command-center layouts were drawn at this scale: at 100% the
 * app reads as zoomed in next to the mockups, and 90% is where they line up.
 *
 * Native webview zoom (what ⌘− does), not CSS `zoom`: it scales everything uniformly and keeps
 * pointer coordinates true, which the terminal's hit-testing and the virtualized lists depend on.
 */
export const APP_ZOOM = 0.9;

/** Applies {@link APP_ZOOM} to the desktop window. A browser (no Tauri) keeps its own zoom. */
export function applyAppZoom(): void {
  if (!isTauri()) return;
  // Imported on demand so the webview API stays out of the eager chunk (`code-splitting.test.tsx`).
  import("@tauri-apps/api/webview")
    .then(({ getCurrentWebview }) => getCurrentWebview().setZoom(APP_ZOOM))
    .catch(() => {
      // An older capability set without the zoom permission: stay at 100% rather than fail start-up.
    });
}
