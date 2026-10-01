import React from "react";
import { Plus } from "lucide-react";
import {
  Button,
  DataTable,
  Spinner,
  type DataTableColumn,
  type DataTableRowAction,
} from "@ivy-interactive/components/ui";
import { bridge } from "../../../api/bridge";
import { useTranslation } from "../../../i18n";
import { notificationsStore } from "../../../state/notificationsStore";
import { describeBridgeError } from "../../../types/api";
import type { ProjectMemoryEntry } from "../../../types/projectAssets";
import { EditProjectMemorySheet } from "../../sheets/EditProjectMemorySheet";
import { SaveError, SubSection } from "../fields";
import type { RemovalRequest } from "../useRemovalConfirm";

export interface ProjectMemorySectionProps {
  projectName: string;
  /** The project screen's shared removal confirm, so a memory delete asks like every other row. */
  requestRemoval: (request: RemovalRequest) => void;
}

/**
 * V1's `ProjectMemoryTableView` (`Apps/Settings/Blades/ProjectTableViews.cs:7`): the markdown files
 * under `<TENDRIL_HOME>/Projects/<Project>/Memory/`, each with its two-line snippet, plus *Add
 * Project Memory* and per-row Edit/Delete. Add and Edit open `EditProjectMemorySheet`.
 *
 * Unlike every other table on this screen, these rows are files rather than `config.yaml` entries,
 * so they are read through `GET /api/projects/:name/memory` and re-read after each write instead of
 * arriving with the project. Delete removes the file, which the confirm's consequence says.
 */
export const ProjectMemorySection: React.FC<ProjectMemorySectionProps> = ({
  projectName,
  requestRemoval,
}) => {
  const { t } = useTranslation("settingsProjects");
  const [files, setFiles] = React.useState<ProjectMemoryEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState<{ fileName: string | null } | null>(null);

  const load = React.useCallback(async () => {
    try {
      setFiles(await bridge.listProjectMemory(projectName));
      setError(null);
    } catch (err) {
      setFiles([]);
      setError(t("memory.loadError", { error: describeBridgeError(err) }));
    }
  }, [projectName, t]);

  React.useEffect(() => {
    setFiles(null);
    void load();
  }, [load]);

  // Saved by an agent in the last day: flagged so a reviewer sees what is new since they looked.
  const recent = (row: ProjectMemoryEntry) =>
    !!row.updated &&
    row.source !== "user" &&
    Date.now() - new Date(row.updated).getTime() < 24 * 60 * 60 * 1000;

  const columns: DataTableColumn<ProjectMemoryEntry>[] = [
    {
      name: "kind",
      header: t("memory.columns.kind"),
      accessor: (row) => row.kind ?? "note",
      cell: (_value, row) => (
        <span className="rounded-md border border-border bg-muted/40 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
          {row.kind ?? "note"}
        </span>
      ),
    },
    {
      name: "snippet",
      header: t("memory.columns.snippet"),
      accessor: (row) => row.snippet,
      cell: (_value, row) => (
        <span className="flex min-w-0 flex-col gap-0.5" title={row.snippet}>
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate">{row.title ?? row.fileName}</span>
            {recent(row) && (
              <span className="shrink-0 rounded-full bg-primary/15 px-1.5 text-[10.5px] font-medium text-success">
                {t("memory.new")}
              </span>
            )}
            {row.stale && (
              <span
                className="shrink-0 rounded-full bg-warning/15 px-1.5 text-[10.5px] font-medium text-warning"
                title={t("memory.staleHint")}
              >
                {t("memory.stale")}
              </span>
            )}
          </span>
          <span className="block max-w-[32rem] truncate text-xs text-muted-foreground">{row.snippet}</span>
        </span>
      ),
    },
    {
      name: "source",
      header: t("memory.columns.source"),
      accessor: (row) => row.source ?? "user",
      cell: (_value, row) => (
        <span className="font-mono text-[11px] text-muted-foreground">{row.source ?? "user"}</span>
      ),
    },
  ];

  const rowActions: DataTableRowAction<ProjectMemoryEntry>[] = [
    { tag: "edit", label: t("rowActions.edit") },
    { tag: "delete", label: t("common:actions.delete"), variant: "destructive" },
  ];

  const remove = async (fileName: string) => {
    try {
      await bridge.deleteProjectMemory(projectName, fileName);
      notificationsStore.notifySuccess(
        t("notifications.saved"),
        t("memory.deleted", { name: fileName }),
      );
    } catch (err) {
      setError(t("saveError", { error: describeBridgeError(err) }));
    }
    await load();
  };

  return (
    <SubSection
      title={t("memory.title")}
      hint={t("memory.hint", { path: "<TENDRIL_HOME>/Projects/<Project>/Memory/" })}
      count={files?.length}
      testId="project-memory"
      action={
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => setEditing({ fileName: null })}
          data-testid="project-memory-add"
        >
          <Plus className="size-4" aria-hidden />
          {t("memory.addButton")}
        </Button>
      }
    >
      {files === null ? (
        <Spinner size="sm" />
      ) : (
        <DataTable<ProjectMemoryEntry>
          // Two text buttons need the room `ProjectDetailBody`'s `EDIT_DELETE_ACTIONS_WIDTH` explains.
          className="[&_table.ivy-data-table]:table-fixed [--ivy-data-table-actions-width:--spacing(36)]"
          data-testid="project-memory-table"
          paginated={false}
          columns={columns}
          rows={files}
          getRowId={(row) => row.fileName}
          rowActions={rowActions}
          emptyState={<span className="text-muted-foreground">{t("memory.empty")}</span>}
          onRowAction={({ tag, row }) => {
            if (tag === "edit") {
              setEditing({ fileName: row.fileName });
            } else if (tag === "delete") {
              requestRemoval({
                kindId: "memoryFile",
                name: row.fileName,
                consequence: t("memory.removal.consequence"),
                onConfirm: () => void remove(row.fileName),
              });
            }
          }}
        />
      )}
      <SaveError message={error} />

      <EditProjectMemorySheet
        open={editing !== null}
        onClose={() => setEditing(null)}
        projectName={projectName}
        fileName={editing?.fileName ?? null}
        onSaved={(fileName) => {
          notificationsStore.notifySuccess(
            t("notifications.saved"),
            t("memory.saved", { name: fileName }),
          );
          void load();
        }}
      />
    </SubSection>
  );
};
