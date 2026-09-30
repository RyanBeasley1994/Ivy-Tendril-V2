import { invoke } from "@tauri-apps/api/core";

/** Which server the desktop app is using. Never carries the password or session token. */
export interface RemoteConnection {
  /** True when the app is pointed at a remote server rather than the local one. */
  active: boolean;
  /** `ssh://user@host` for an SSH connection, the server's origin otherwise. */
  url: string | null;
  username: string | null;
  /** The last login succeeded, so requests are carrying a valid session token. */
  authenticated: boolean;
  /** Why the last login failed, when it did. */
  error: string | null;
}

export interface RemoteConnectRequest {
  /** `host`, `host:port` or `user@host[:port]`. */
  sshAddress: string;
  sshUsername: string;
  sshPassword: string;
  /** The daemon's port on the server; 5010 when omitted. */
  remotePort?: number;
  /** The Tendril login on the server. */
  username: string;
  password: string;
}

/**
 * Connecting the desktop app to a Tendril server on another machine (`service::remote` in
 * src-tauri). Connect and disconnect both restart the app on success, so their promises resolve
 * just before the window reloads.
 */
export const remoteApi = {
  async getConnection(this: void): Promise<RemoteConnection> {
    return invoke<RemoteConnection>("cmd_get_remote_connection");
  },
  /** Tunnels in over SSH, then logs in to Tendril through the tunnel. */
  async connect(this: void, request: RemoteConnectRequest): Promise<void> {
    return invoke<void>("cmd_connect_remote", { ...request });
  },
  async disconnect(this: void): Promise<void> {
    return invoke<void>("cmd_disconnect_remote");
  },
};

export type RemoteApi = typeof remoteApi;
