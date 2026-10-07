import React from "react";
import { Button, Callout, Spinner } from "@ivy-interactive/components/ui";
import { Smartphone } from "lucide-react";
import { useTranslation } from "../../i18n";
import { isStandalone } from "../../pwa/register";
import { isTauri } from "../../utils/tauri";
import { SettingsSection } from "./fields";

type Availability = "ready" | "insecure" | "needs-install" | "unsupported";

const isIos = () => /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);

function availability(): Availability {
  if (!window.isSecureContext) return "insecure";
  if (!("serviceWorker" in navigator)) return "unsupported";
  // iOS only offers push to a web app that was added to the Home Screen.
  if (!("PushManager" in window)) return isIos() && !isStandalone() ? "needs-install" : "unsupported";
  return "ready";
}

const keyBytes = (b64url: string): Uint8Array => {
  const padded = b64url.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(b64url.length / 4) * 4, "=");
  const raw = atob(padded);
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
};

async function post(path: string, body?: unknown): Promise<Response> {
  const response = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    const detail = await response.json().then((j: { error?: string }) => j.error, () => undefined);
    throw new Error(detail || `${response.status}`);
  }
  return response;
}

/**
 * Turns browser push on for this device, so the installed web app on a phone buzzes when a manager
 * needs you or its work is done, even with the app closed. The daemon sends; this only subscribes.
 */
export const PhoneNotificationsSection: React.FC = () => {
  const { t } = useTranslation("settings");
  const state = React.useMemo(availability, []);
  const [subscribed, setSubscribed] = React.useState<boolean | null>(null);
  const [permission, setPermission] = React.useState<NotificationPermission>(() =>
    "Notification" in window ? Notification.permission : "default",
  );
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (state !== "ready") return;
    let live = true;
    navigator.serviceWorker.ready
      .then((reg) => reg.pushManager.getSubscription())
      .then((sub) => live && setSubscribed(sub !== null))
      .catch(() => live && setSubscribed(false));
    return () => {
      live = false;
    };
  }, [state]);

  if (isTauri()) return null;

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await work();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const enable = () =>
    run(async () => {
      const result = await Notification.requestPermission();
      setPermission(result);
      if (result !== "granted") return;
      const reg = await navigator.serviceWorker.ready;
      const status = (await (await fetch("/api/push/status")).json()) as { publicKey: string };
      const sub = await reg.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: keyBytes(status.publicKey) as BufferSource,
      });
      const json = sub.toJSON() as { endpoint: string; keys: { p256dh: string; auth: string } };
      await post("/api/push/subscribe", { ...json, label: isIos() ? "iPhone" : navigator.platform || "Browser" });
      setSubscribed(true);
    });

  const disable = () =>
    run(async () => {
      const reg = await navigator.serviceWorker.ready;
      const sub = await reg.pushManager.getSubscription();
      if (sub) {
        await post("/api/push/unsubscribe", { endpoint: sub.endpoint });
        await sub.unsubscribe();
      }
      setSubscribed(false);
    });

  const test = () =>
    run(async () => {
      const result = (await (await post("/api/push/test")).json()) as { delivered: number };
      setMessage(result.delivered > 0 ? t("phoneNotifications.testSent") : t("phoneNotifications.testNone"));
    });

  return (
    <SettingsSection title={t("phoneNotifications.title")} hint={t("phoneNotifications.hint")} testId="phone-notifications-card">
      <div className="space-y-3">
        {state === "insecure" && <Callout.Warning>{t("phoneNotifications.insecure")}</Callout.Warning>}
        {state === "needs-install" && <Callout.Info>{t("phoneNotifications.needsInstall")}</Callout.Info>}
        {state === "unsupported" && <Callout.Info>{t("phoneNotifications.unsupported")}</Callout.Info>}
        {state === "ready" && permission === "denied" && <Callout.Warning>{t("phoneNotifications.denied")}</Callout.Warning>}
        {error && <p className="text-xs text-destructive">{error}</p>}
        {message && <p className="text-xs text-muted-foreground">{message}</p>}

        {state === "ready" && (
          <div className="flex flex-wrap items-center gap-3">
            <Smartphone className="size-4 text-primary" aria-hidden />
            <span className="text-sm">
              {subscribed ? t("phoneNotifications.on") : t("phoneNotifications.off")}
            </span>
            {subscribed ? (
              <>
                <Button type="button" variant="outline" disabled={busy} onClick={() => void test()}>
                  {t("phoneNotifications.test")}
                </Button>
                <Button type="button" variant="outline" disabled={busy} onClick={() => void disable()}>
                  {t("phoneNotifications.turnOff")}
                </Button>
              </>
            ) : (
              <Button type="button" disabled={busy || permission === "denied" || subscribed === null} onClick={() => void enable()}>
                {busy && <Spinner size="sm" aria-hidden />}
                {t("phoneNotifications.turnOn")}
              </Button>
            )}
          </div>
        )}
      </div>
    </SettingsSection>
  );
};
