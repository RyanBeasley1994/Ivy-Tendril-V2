import React from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderBrowserDialog } from "../dialogs/FolderBrowserDialog";
import { useTranslation } from "../../i18n";
import { serviceStore } from "../../state/serviceStore";
import { isTauri } from "../../utils/tauri";

/**
 * The Browse behind every repository picker: the OS folder dialog in the desktop app on its own
 * daemon, and the daemon-side {@link FolderBrowserDialog} anywhere else.
 *
 * The split is about *whose* disk is shown. The desktop app and a local daemon share a machine, so
 * the native picker is both nicer and correct. A browser reaching the daemon over a tunnel, or the
 * desktop app connected to a remote server, would get a native picker for its own machine, whose
 * paths mean nothing to the daemon that has to open them.
 *
 * `browse` resolves with the chosen path, or `null` when the operator backed out. Render `dialog`
 * somewhere in the caller's tree.
 */
export function useRepoFolderPicker(): {
  browse: (initialPath?: string) => Promise<string | null>;
  dialog: React.ReactNode;
} {
  const { t } = useTranslation("onboarding");
  const [pending, setPending] = React.useState<{
    initialPath?: string;
    resolve: (path: string | null) => void;
  } | null>(null);

  const browse = React.useCallback(
    async (initialPath?: string): Promise<string | null> => {
      if (usesNativePicker()) {
        const selected = await open({
          directory: true,
          multiple: false,
          title: t("firstProject.browseTitle"),
          defaultPath: initialPath || undefined,
        });
        return typeof selected === "string" && selected ? selected : null;
      }
      return new Promise((resolve) => setPending({ initialPath, resolve }));
    },
    [t],
  );

  const settle = (path: string | null) => {
    pending?.resolve(path);
    setPending(null);
  };

  // Always mounted: whether the daemon is remote is only known once its info loads, so the dialog
  // can't be ruled out at render time. It stays closed until `browse` opens it.
  const dialog = (
    <FolderBrowserDialog
      isOpen={pending !== null}
      initialPath={pending?.initialPath}
      onClose={() => settle(null)}
      onSelect={(path) => settle(path)}
    />
  );

  return { browse, dialog };
}

/** The native picker shows this machine's disk, which is only the daemon's when the daemon is local. */
function usesNativePicker(): boolean {
  return isTauri() && serviceStore.getState().info?.ownership !== "Remote";
}
