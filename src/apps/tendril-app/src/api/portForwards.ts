import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "../utils/tauri";

/** A port on the server opened on this machine: `service::port_forward` in src-tauri. */
export interface PortForward {
  remotePort: number;
  localPort: number;
  /** `false` when the daemon runs here and nothing needed forwarding. */
  forwarded: boolean;
}

/**
 * `ssh -L` over the Forge connection. Desktop only: a browser cannot listen on a port, so there the
 * port is answered unchanged, which is right for a daemon on the same machine as the browser.
 */
export const portForwardsApi = {
  async forward(this: void, port: number): Promise<PortForward> {
    if (!isTauri()) return { remotePort: port, localPort: port, forwarded: false };
    const fwd = await invoke<PortForward>("cmd_forward_port", { port });
    void refreshPortForwards();
    return fwd;
  },
  async list(this: void): Promise<PortForward[]> {
    if (!isTauri()) return [];
    return invoke<PortForward[]>("cmd_list_port_forwards");
  },
  async stop(this: void, port: number): Promise<boolean> {
    if (!isTauri()) return false;
    const stopped = await invoke<boolean>("cmd_stop_port_forward", { port });
    void refreshPortForwards();
    return stopped;
  },
};

// The open forwards, shared by the top bar's chip, its modal and Settings, and refreshed after every
// change - including one a clicked `localhost` link made - so all three agree.
let current: PortForward[] = [];
const listeners = new Set<() => void>();

export async function refreshPortForwards(): Promise<void> {
  try {
    current = await portForwardsApi.list();
  } catch {
    return;
  }
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  if (listeners.size === 1) void refreshPortForwards();
  return () => {
    listeners.delete(l);
  };
};

export function usePortForwards(): PortForward[] {
  return useSyncExternalStore(subscribe, () => current, () => current);
}
