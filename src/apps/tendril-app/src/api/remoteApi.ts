import { invoke } from "@tauri-apps/api/core";

/** Which server the desktop app is using. Never carries the password or session token. */
export interface RemoteConnection {
  /** True when the app is pointed at a remote server rather than the local one. */
  active: boolean;
  url: string | null;
  username: string | null;
  /** The last login succeeded, so requests are carrying a valid session token. */
  authenticated: boolean;
  /** Why the last login failed, when it did. */
  error: string | null;
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
  async connect(this: void, url: string, username: string, password: string): Promise<void> {
    return invoke<void>("cmd_connect_remote", { url, username, password });
  },
  async disconnect(this: void): Promise<void> {
    return invoke<void>("cmd_disconnect_remote");
  },
};

export type RemoteApi = typeof remoteApi;
