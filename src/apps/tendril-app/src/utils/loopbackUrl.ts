import { portForwardsApi } from "../api/portForwards";

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "0.0.0.0", "[::1]", "::1"]);

/** The port of a loopback http(s) URL a dev server printed, when it is one worth forwarding. */
export function loopbackPort(url: string): number | null {
  try {
    const parsed = new URL(url);
    if (!/^https?:$/.test(parsed.protocol) || !LOOPBACK_HOSTS.has(parsed.hostname)) return null;
    const port = Number(parsed.port || (parsed.protocol === "https:" ? 443 : 80));
    return port >= 1024 ? port : null;
  } catch {
    return null;
  }
}

/**
 * The URL to open here for a loopback URL on the server: forwarded through the Forge connection when
 * the app is connected to a remote server, so `http://localhost:5173` printed on the server opens
 * the server's dev server. Anything else, or a failed forward, is returned as it was.
 */
export async function forwardedUrl(url: string): Promise<string> {
  const port = loopbackPort(url);
  if (port === null) return url;
  try {
    const fwd = await portForwardsApi.forward(port);
    if (!fwd.forwarded) return url;
    const parsed = new URL(url);
    parsed.hostname = "127.0.0.1";
    parsed.port = String(fwd.localPort);
    return parsed.toString();
  } catch {
    return url;
  }
}
