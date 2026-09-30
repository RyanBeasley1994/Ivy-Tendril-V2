import React from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderBrowserDialog } from "../dialogs/FolderBrowserDialog";
import { useTranslation } from "../../i18n";
import { isTauri } from "../../utils/tauri";

/**
 * The Browse behind every repository picker: the OS folder dialog in the desktop app, and the
 * daemon-side {@link FolderBrowserDialog} anywhere else.
 *
 * The split is about *whose* disk is shown. The desktop app and its daemon share a machine, so the
 * native picker is both nicer and correct. A browser reaching the daemon over a tunnel would get a
 * native picker for its own machine, whose paths mean nothing to the daemon that has to open them.
 *
 * `browse` resolves with the chosen path, or `null` when the operator backed out. Render `dialog`
 * somewhere in the caller's tree; it is `null` in the desktop app.
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
      if (isTauri()) {
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

  const dialog = isTauri() ? null : (
    <FolderBrowserDialog
      isOpen={pending !== null}
      initialPath={pending?.initialPath}
      onClose={() => settle(null)}
      onSelect={(path) => settle(path)}
    />
  );

  return { browse, dialog };
}
