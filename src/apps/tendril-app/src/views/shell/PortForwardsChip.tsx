import React from "react";
import { Cable } from "lucide-react";
import { DialogShell } from "@ivy-interactive/components/dialogs";
import { usePortForwards } from "../../api/portForwards";
import { useTranslation } from "../../i18n";
import { PortForwardsPanel } from "../settings/PortForwardsPanel";

/**
 * The top bar's port-forward status, beside the service chip: how many server ports are open on this
 * machine, green while any are, and a modal to add, open and stop them. Only shown while connected to
 * a remote server, where forwarding means something.
 */
export const PortForwardsChip: React.FC = () => {
  const { t } = useTranslation("common");
  const { t: ts } = useTranslation("settings");
  const forwards = usePortForwards();
  const [open, setOpen] = React.useState(false);
  const active = forwards.length > 0;

  return (
    <>
      <button
        type="button"
        data-testid="port-forwards-chip"
        title={t("topBar.forwardsTooltip")}
        onClick={() => setOpen(true)}
        className="flex h-[26px] items-center gap-1.5 rounded-[7px] border border-border bg-muted px-2.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
      >
        <Cable className={`size-3.5 ${active ? "text-success" : ""}`} aria-hidden="true" />
        {active
          ? t("topBar.forwards", { count: forwards.length, ports: forwards.map((f) => f.remotePort).join(" ") })
          : t("topBar.forwardsNone")}
      </button>
      <DialogShell
        isOpen={open}
        onClose={() => setOpen(false)}
        title={ts("remoteServer.forwards.title")}
        testId="port-forwards-dialog"
        width="rem32"
      >
        <PortForwardsPanel bare />
      </DialogShell>
    </>
  );
};
