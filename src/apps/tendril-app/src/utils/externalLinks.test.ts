import { describe, expect, it } from "vitest";
import { isExternalHref } from "./externalLinks";

describe("isExternalHref", () => {
  const app = "tauri://localhost";
  const devApp = "http://127.0.0.1:5173";

  it("sends web, mail and phone links out", () => {
    expect(isExternalHref("https://github.com/org/repo/pull/1", app)).toBe(true);
    expect(isExternalHref("http://localhost:3000", app)).toBe(true);
    expect(isExternalHref("mailto:someone@example.com", app)).toBe(true);
    expect(isExternalHref("tel:+15551234", app)).toBe(true);
  });

  it("keeps the app's own routes, even when the app is served over http in development", () => {
    expect(isExternalHref("#/plans/42", app)).toBe(false);
    expect(isExternalHref("/settings", devApp)).toBe(false);
    expect(isExternalHref("http://127.0.0.1:5173/plans", devApp)).toBe(false);
    // Another port on the same host is a different app: the daemon, or a dev server.
    expect(isExternalHref("http://127.0.0.1:3000", devApp)).toBe(true);
  });

  it("keeps what the app made itself, and anything unparseable", () => {
    expect(isExternalHref("blob:tauri://localhost/abc", app)).toBe(false);
    expect(isExternalHref("data:text/plain,hi", app)).toBe(false);
    expect(isExternalHref("javascript:void(0)", app)).toBe(false);
    expect(isExternalHref("http://[bad", app)).toBe(false);
  });
});
