import React from "react";
import { GitRepos } from "./GitRepos";
import { RepoWorkspace } from "./RepoWorkspace";

/** The Git page: every repository as a card, or one repository's workspace when its id is in the address. */
export const GitView: React.FC<{
  repoId?: string;
  project?: string;
  onOpenRepo: (id: string) => void;
  onBack: () => void;
  onAskManager: (project: string, text: string) => void;
}> = ({ repoId, project, onOpenRepo, onBack, onAskManager }) =>
  repoId ? (
    <RepoWorkspace repoId={repoId} onBack={onBack} onOpenRepo={onOpenRepo} onAskManager={onAskManager} />
  ) : (
    <GitRepos projectFilter={project} onOpenRepo={onOpenRepo} />
  );
