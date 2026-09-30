import React from "react";
import { DialogShell } from "@ivy-interactive/components/dialogs";
import { bridge } from "../../api/bridge";
import { useTranslation } from "../../i18n";
import type { TendrilConfig } from "../../types/api";
import { AddProjectView } from "../settings/AddProjectView";
import { readProjectEntries } from "../settings/projectConfig";

export interface AddProjectDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /** Names already taken, for the duplicate check. */
  existingNames: string[];
  /**
   * The project exists. Fires as soon as the row is written, not at Finish, so the dialog underneath
   * can offer and select it while the setup agent is still running here.
   */
  onCreated: (name: string) => void;
}

/**
 * Settings' Add Project flow (name and repositories, the setup agent, then the harness it produced)
 * as a modal, so the Create Plan / Mission dialog can add a project without leaving for Settings.
 * The steps are {@link AddProjectView}'s; this only supplies what `SettingsView` otherwise does:
 * the create call and a config read for the harness step.
 */
export const AddProjectDialog: React.FC<AddProjectDialogProps> = ({
  isOpen,
  onClose,
  existingNames,
  onCreated,
}) => {
  const { t } = useTranslation("onboarding");
  const [config, setConfig] = React.useState<TendrilConfig | null>(null);
  const [createdName, setCreatedName] = React.useState<string | null>(null);

  const reloadConfig = React.useCallback(async () => {
    try {
      setConfig(await bridge.getConfig());
    } catch {
      // The harness step shows the project unconfigured rather than failing the dialog.
    }
  }, []);

  const createdProject = React.useMemo(() => {
    if (!createdName) return null;
    return (
      readProjectEntries(config).find(
        (project) => project.name.toLowerCase() === createdName.toLowerCase(),
      ) ?? null
    );
  }, [config, createdName]);

  return (
    <DialogShell
      isOpen={isOpen}
      onClose={onClose}
      title={t("addProject.title")}
      testId="add-project-dialog"
      width="rem40"
    >
      <AddProjectView
        existingNames={existingNames}
        onCreate={async (name, repos) => {
          const created = await bridge.createProject({ name, repos });
          await reloadConfig();
          setCreatedName(name);
          onCreated(name);
          // A remote was cloned into TENDRIL_HOME; the response is where the caller learns the path.
          return created?.repos?.map((repo) => repo.path) ?? repos;
        }}
        createdProject={createdProject}
        onFinish={onClose}
        onReloadConfig={reloadConfig}
      />
    </DialogShell>
  );
};
