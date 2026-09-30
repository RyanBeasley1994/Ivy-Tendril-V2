import React from "react";
import { ChevronDown, Github, Lock } from "lucide-react";
import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Spinner,
} from "@ivy-interactive/components/ui";
import { bridge } from "../../api/bridge";
import { describeBridgeError, type GitHubRepo } from "../../types/api";
import { useTranslation } from "../../i18n";

/**
 * Held for the session: the list is one paginated `gh api` call, slow enough on a big account that
 * reopening the dropdown should not repeat it. A failure is not cached, so fixing `gh auth` and
 * reopening tries again.
 */
let cachedRepos: GitHubRepo[] | null = null;

export interface GitHubRepoPickerProps {
  /** A repository was chosen; given its clone URL. */
  onPick: (cloneUrl: string) => void;
  /** Clone URLs already in the project, shown as added rather than offered again. */
  added?: string[];
}

/**
 * "Clone from GitHub": a searchable dropdown of every repository the daemon's `gh` user can clone.
 * Picking one hands its HTTPS URL to the repository list, which the project create then clones - the
 * same path a pasted URL takes.
 */
export const GitHubRepoPicker: React.FC<GitHubRepoPickerProps> = ({ onPick, added = [] }) => {
  const { t } = useTranslation("onboarding");
  const [open, setOpen] = React.useState(false);
  const [repos, setRepos] = React.useState<GitHubRepo[] | null>(cachedRepos);
  const [error, setError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(false);

  const load = React.useCallback(async () => {
    if (cachedRepos) return;
    setLoading(true);
    setError(null);
    try {
      const list = await bridge.listGitHubRepos();
      cachedRepos = list;
      setRepos(list);
    } catch (err) {
      setError(describeBridgeError(err));
    } finally {
      setLoading(false);
    }
  }, []);

  const addedSet = new Set(added.map((url) => url.toLowerCase()));

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) void load();
      }}
    >
      <PopoverTrigger asChild>
        <Button type="button" variant="outline" data-testid="add-project-github">
          <Github className="size-4" aria-hidden />
          {t("repoPicker.github.trigger")}
          <ChevronDown className="size-4 opacity-60" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[26rem] p-0">
        <Command>
          <CommandInput placeholder={t("repoPicker.github.search")} />
          <CommandList className="max-h-80">
            {loading && (
              <div className="flex items-center gap-2 px-3 py-4 text-sm text-muted-foreground">
                <Spinner size="sm" aria-hidden />
                {t("repoPicker.github.loading")}
              </div>
            )}
            {error && (
              <div
                className="px-3 py-4 text-xs text-destructive"
                data-testid="add-project-github-error"
              >
                {error}
              </div>
            )}
            {!loading && !error && repos && (
              <>
                <CommandEmpty>{t("repoPicker.github.empty")}</CommandEmpty>
                <CommandGroup>
                  {repos.map((repo) => {
                    const isAdded = addedSet.has(repo.cloneUrl.toLowerCase());
                    return (
                      <CommandItem
                        key={repo.fullName}
                        value={`${repo.fullName} ${repo.description ?? ""}`}
                        disabled={isAdded}
                        onSelect={() => {
                          onPick(repo.cloneUrl);
                          setOpen(false);
                        }}
                        className="flex items-start gap-2"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5">
                            <span className="truncate font-mono text-xs text-foreground">
                              {repo.fullName}
                            </span>
                            {repo.isPrivate && (
                              <Lock
                                className="size-3 shrink-0 text-muted-foreground"
                                aria-label={t("repoPicker.github.private")}
                              />
                            )}
                          </div>
                          {repo.description && (
                            <p className="truncate text-xs text-muted-foreground">
                              {repo.description}
                            </p>
                          )}
                        </div>
                        {isAdded && (
                          <span className="shrink-0 text-xs text-muted-foreground">
                            {t("repoPicker.github.added")}
                          </span>
                        )}
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
};
