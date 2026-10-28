import React from "react";
import { X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { ProjectSummary } from "../../types/api";
import { supportApi } from "../../api/support";
import { supportDock, useSupportDock, type DockedTicket } from "../../state/supportDock";
import { TicketPanel } from "./SupportView";

/**
 * Minimised support tickets, on every page: a row of bubbles at the bottom right, and the one that is
 * open as a small floating ticket window above them. Minimise sends it back to its bubble; the cross
 * on a bubble undocks it.
 */
export const SupportDock: React.FC<{ projects: ProjectSummary[]; onOpenOnSupportPage: (ticketId: string) => void }> = ({
  projects,
  onOpenOnSupportPage,
}) => {
  const { docked, openId } = useSupportDock();
  const unread = useDockedUnread(docked);
  if (docked.length === 0) return null;
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-40 flex flex-col items-end gap-3" data-testid="support-dock">
      {openId && (
        <div className="pointer-events-auto h-[min(620px,calc(100dvh-7rem))] w-[min(440px,calc(100vw-2rem))]">
          <TicketPanel
            key={openId}
            id={openId}
            projects={projects}
            variant="floating"
            onChanged={() => undefined}
            onClose={() => supportDock.remove(openId)}
            onMinimize={() => supportDock.collapse()}
            onExpand={() => {
              supportDock.remove(openId);
              onOpenOnSupportPage(openId);
            }}
          />
        </div>
      )}
      <div className="pointer-events-auto flex flex-row-reverse gap-2">
        {docked.map((t) => (
          <Bubble key={t.id} ticket={t} open={openId === t.id} unread={unread[t.id] ?? false} />
        ))}
      </div>
    </div>
  );
};

const initials = (t: DockedTicket) =>
  (t.partner || t.subject)
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase())
    .join("") || "?";

const Bubble: React.FC<{ ticket: DockedTicket; open: boolean; unread: boolean }> = ({ ticket, open, unread }) => (
  <div className="group/bubble relative">
    <button
      type="button"
      onClick={() => supportDock.open(ticket.id)}
      title={`${ticket.subject}${ticket.partner ? ` · ${ticket.partner}` : ""}`}
      aria-label={`Ticket: ${ticket.subject}`}
      aria-pressed={open}
      className={cn(
        "flex size-12 items-center justify-center rounded-full border text-[13px] font-semibold shadow-[0_8px_24px_rgba(0,0,0,0.45)] transition-transform hover:scale-105",
        open ? "border-primary bg-primary text-primary-foreground" : "border-border bg-card text-foreground",
      )}
    >
      {initials(ticket)}
    </button>
    {unread && !open && (
      <span className="absolute right-0 top-0 size-3 rounded-full border-2 border-background bg-primary" aria-label="New reply" />
    )}
    <button
      type="button"
      onClick={() => supportDock.remove(ticket.id)}
      aria-label={`Close ${ticket.subject}`}
      className="absolute -left-1 -top-1 hidden size-5 items-center justify-center rounded-full border border-border bg-background text-muted-foreground hover:text-foreground group-hover/bubble:inline-flex"
    >
      <X className="size-3" aria-hidden="true" />
    </button>
  </div>
);

/** Which docked tickets have a partner reply waiting, from the board's own `hasUnread`. */
const useDockedUnread = (docked: DockedTicket[]): Record<string, boolean> => {
  const [unread, setUnread] = React.useState<Record<string, boolean>>({});
  const key = docked.map((t) => t.id).join(",");
  React.useEffect(() => {
    if (!key) return;
    let live = true;
    const ids = new Set(key.split(","));
    const load = () =>
      supportApi
        .list(true)
        .then((res) => {
          if (!live) return;
          setUnread(Object.fromEntries(res.tickets.filter((t) => ids.has(t.id)).map((t) => [t.id, !!t.hasUnread])));
        })
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, 30_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [key]);
  return unread;
};
