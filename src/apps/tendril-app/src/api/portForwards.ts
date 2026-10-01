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
    return invoke<PortForward>("cmd_forward_port", { port });
  },
  async list(this: void): Promise<PortForward[]> {
    if (!isTauri()) return [];
    return invoke<PortForward[]>("cmd_list_port_forwards");
  },
  async stop(this: void, port: number): Promise<boolean> {
    if (!isTauri()) return false;
    return invoke<boolean>("cmd_stop_port_forward", { port });
  },
};
