import * as React from "react";
import { ArrowUp, Eye, EyeOff, Folder, FolderGit2, Home } from "lucide-react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Spinner,
} from "@ivy-interactive/components/ui";
import { bridge } from "../../api/bridge";
import type { DirectoryListing } from "../../types/api";
import { useTranslation } from "../../i18n";

export interface FolderBrowserDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /** Called with the chosen folder's absolute path on the daemon's host. */
  onSelect: (path: string) => void;
  /** Where to open; the daemon user's home directory when absent. */
  initialPath?: string;
}

/**
 * A folder picker over the daemon's disk, for when the native one would show the wrong machine.
 *
 * The desktop app shares a host with its daemon, so it uses the OS picker. A client reaching the
 * daemon over a tunnel does not: the repository it is registering lives next to the daemon, and only
 * `GET /api/fs/directories` can see there. Folders holding a `.git` are marked, since that is
 * almost always what the operator is looking for.
 *
 * Listings are sequenced so a slow reply for a folder the operator has already left is dropped
 * rather than shown under the wrong path.
 */
export function FolderBrowserDialog({
  isOpen,
  onClose,
  onSelect,
  initialPath,
}: FolderBrowserDialogProps) {
  const { t } = useTranslation("onboarding");
  const [listing, setListing] = React.useState<DirectoryListing | null>(null);
  const [pathDraft, setPathDraft] = React.useState("");
  const [showHidden, setShowHidden] = React.useState(false);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const request = React.useRef(0);

  const load = React.useCallback(async (path: string | undefined, hidden: boolean) => {
    const seq = ++request.current;
    setLoading(true);
    setError(null);
    try {
      const next = await bridge.listDirectories(path, hidden);
      if (seq !== request.current) return;
      setListing(next);
      setPathDraft(next.path);
    } catch (err) {
      if (seq !== request.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === request.current) setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    if (!isOpen) return;
    void load(initialPath?.trim() || undefined, showHidden);
    // Only on open: later navigation is driven by the operator, not by a changed prop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  const current = listing?.path;
  const go = (path: string | undefined) => void load(path, showHidden);

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-2xl" data-testid="folder-browser-dialog">
        <DialogHeader>
          <DialogTitle>{t("folderBrowser.title")}</DialogTitle>
          <DialogDescription>{t("folderBrowser.description")}</DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-3 px-6 pb-2">
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              aria-label={t("folderBrowser.up")}
              disabled={!listing?.parent || loading}
              onClick={() => go(listing?.parent ?? undefined)}
              data-testid="folder-browser-up"
            >
              <ArrowUp className="size-4" aria-hidden />
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              aria-label={t("folderBrowser.home")}
              disabled={loading}
              onClick={() => go(undefined)}
              data-testid="folder-browser-home"
            >
              <Home className="size-4" aria-hidden />
            </Button>
            <Input
              aria-label={t("folderBrowser.pathLabel")}
              value={pathDraft}
              onChange={(e) => setPathDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return;
                e.preventDefault();
                go(pathDraft.trim() || undefined);
              }}
              className="font-mono text-xs"
              data-testid="folder-browser-path"
            />
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-pressed={showHidden}
              aria-label={t(showHidden ? "folderBrowser.hideHidden" : "folderBrowser.showHidden")}
              onClick={() => {
                const next = !showHidden;
                setShowHidden(next);
                void load(current, next);
              }}
              data-testid="folder-browser-hidden"
            >
              {showHidden ? (
                <Eye className="size-4" aria-hidden />
              ) : (
                <EyeOff className="size-4" aria-hidden />
              )}
            </Button>
          </div>

          {listing && listing.roots.length > 1 && (
            <div className="flex flex-wrap gap-1">
              {listing.roots.map((root) => (
                <Button
                  key={root}
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="font-mono text-xs"
                  onClick={() => go(root)}
                >
                  {root}
                </Button>
              ))}
            </div>
          )}

          {error && (
            <p className="text-xs text-destructive" data-testid="folder-browser-error">
              {error}
            </p>
          )}

          <div className="h-80 overflow-y-auto rounded-box border border-border">
            {loading && !listing ? (
              <div className="flex h-full items-center justify-center">
                <Spinner size="lg" aria-hidden />
              </div>
            ) : listing && listing.entries.length === 0 ? (
              <p className="p-3 text-xs text-muted-foreground">{t("folderBrowser.empty")}</p>
            ) : (
              <ul className="divide-y divide-border" data-testid="folder-browser-entries">
                {listing?.entries.map((entry) => (
                  <li key={entry.path}>
                    <button
                      type="button"
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-foreground hover:bg-muted/60"
                      onClick={() => go(entry.path)}
                      onDoubleClick={() => entry.isGitRepo && onSelect(entry.path)}
                    >
                      {entry.isGitRepo ? (
                        <FolderGit2 className="size-4 shrink-0 text-primary" aria-hidden />
                      ) : (
                        <Folder className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                      )}
                      <span className="truncate">{entry.name}</span>
                      {entry.isGitRepo && (
                        <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                          {t("folderBrowser.gitRepo")}
                        </span>
                      )}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {listing?.truncated && (
            <p className="text-xs text-muted-foreground">{t("folderBrowser.truncated")}</p>
          )}
        </div>

        <DialogFooter className="px-6 pb-6">
          <Button type="button" variant="outline" onClick={onClose}>
            {t("folderBrowser.cancel")}
          </Button>
          <Button
            type="button"
            disabled={!current || loading}
            onClick={() => current && onSelect(current)}
            data-testid="folder-browser-select"
          >
            {listing?.isGitRepo ? t("folderBrowser.selectRepo") : t("folderBrowser.select")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
