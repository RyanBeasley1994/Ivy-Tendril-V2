import React from "react";
import { X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { formatAge } from "../../utils/commandCenter";
import { GhostButton, PrimaryButton } from "../../components/page/kit";
import type { RepoSummary } from "../../types/git";

/** One colour per lane, cycling. Tuned for the dark theme: all read clearly on the blue-black base. */
export const LANE_COLORS = ["#19e0a5", "#4f86e6", "#b78bf5", "#f0b54a", "#e25c66", "#41c7e0", "#e08b41", "#8fd14f"];
export const laneColor = (index: number): string => LANE_COLORS[((index % LANE_COLORS.length) + LANE_COLORS.length) % LANE_COLORS.length];

export const ago = (unixSeconds: number): string =>
  unixSeconds > 0 ? `${formatAge(Math.max(0, Date.now() - unixSeconds * 1000))} ago` : "";

export const agoIso = (iso: string): string => {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? "" : `${formatAge(Math.max(0, Date.now() - t))} ago`;
};

/** How much a repository needs looking at, for sorting the cards: conflicts first, then loose work. */
export const attentionScore = (s: RepoSummary | null): number => {
  if (!s) return 5;
  if (s.conflicted > 0 || s.state !== "clean") return 4;
  if (s.staged + s.unstaged + s.untracked > 0) return 3;
  if (s.behind > 0) return 2;
  if (s.ahead > 0 || (s.upstream === null && !s.detached && s.branch !== null && s.lastCommit !== null)) return 1;
  return 0;
};

/** A dialog: dimmed backdrop, Escape and a click outside close it. */
export const Modal: React.FC<{
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  footer?: React.ReactNode;
  width?: number;
}> = ({ title, onClose, children, footer, width = 460 }) => {
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/55 px-4 pt-[12vh] backdrop-blur-[3px]"
      role="presentation"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        style={{ width }}
        className="flex max-h-[76vh] max-w-full flex-col overflow-hidden rounded-2xl border border-[#22303d] bg-card shadow-[0_30px_80px_-20px_rgba(0,0,0,0.8)]"
      >
        <div className="flex shrink-0 items-center gap-2 border-b border-border/70 px-5 py-3.5">
          <h2 className="m-0 min-w-0 flex-1 truncate text-[14px] font-semibold text-foreground">{title}</h2>
          <button type="button" aria-label="Close" onClick={onClose} className="rounded-lg p-1 text-muted-foreground hover:bg-secondary hover:text-foreground">
            <X className="size-4" aria-hidden="true" />
          </button>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">{children}</div>
        {footer && <div className="flex shrink-0 justify-end gap-2 border-t border-border/70 px-5 py-3">{footer}</div>}
      </div>
    </div>
  );
};

export const Field: React.FC<{ label: string; hint?: string; children: React.ReactNode }> = ({ label, hint, children }) => (
  <label className="flex flex-col gap-1.5">
    <span className="font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">{label}</span>
    {children}
    {hint && <span className="text-[11.5px] text-muted-foreground">{hint}</span>}
  </label>
);

export const inputClass =
  "h-9 w-full rounded-lg border border-input bg-muted px-3 text-[13px] text-foreground outline-none placeholder:text-muted-foreground focus:border-primary/50";

export const textareaClass =
  "min-h-[88px] w-full resize-y rounded-lg border border-input bg-muted px-3 py-2 text-[13px] text-foreground outline-none placeholder:text-muted-foreground focus:border-primary/50";

/** A yes/no question, with the consequence stated plainly. `danger` makes the button red. */
export const ConfirmDialog: React.FC<{
  title: string;
  body: React.ReactNode;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}> = ({ title, body, confirmLabel, danger, onConfirm, onClose }) => (
  <Modal
    title={title}
    onClose={onClose}
    footer={
      <>
        <GhostButton onClick={onClose}>Cancel</GhostButton>
        {danger ? (
          <button
            type="button"
            onClick={() => {
              onConfirm();
              onClose();
            }}
            className="inline-flex h-[30px] items-center rounded-lg bg-destructive px-3 text-[12.5px] font-semibold text-white hover:opacity-90"
          >
            {confirmLabel}
          </button>
        ) : (
          <PrimaryButton
            onClick={() => {
              onConfirm();
              onClose();
            }}
          >
            {confirmLabel}
          </PrimaryButton>
        )}
      </>
    }
  >
    <div className="text-[13px] leading-relaxed text-muted-foreground">{body}</div>
  </Modal>
);

export interface MenuItem {
  label: string;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
  /** A thin rule above this item. */
  separator?: boolean;
}

/** A right-click menu at the pointer. Closes on any click, Escape, scroll or resize. */
export const ContextMenu: React.FC<{ x: number; y: number; items: MenuItem[]; onClose: () => void }> = ({ x, y, items, onClose }) => {
  React.useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", close, true);
    };
  }, [onClose]);

  // Keep it on screen.
  const left = Math.min(x, window.innerWidth - 230);
  const top = Math.min(y, window.innerHeight - items.length * 32 - 16);

  return (
    <div
      role="menu"
      style={{ left, top }}
      className="fixed z-50 min-w-[210px] rounded-xl border border-[#22303d] bg-card p-1 shadow-[0_18px_50px_-12px_rgba(0,0,0,0.8)]"
      onClick={(e) => e.stopPropagation()}
    >
      {items.map((item) => (
        <React.Fragment key={item.label}>
          {item.separator && <div className="mx-2 my-1 h-px bg-border/70" />}
          <button
            type="button"
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onClick();
            }}
            className={cn(
              "flex w-full items-center rounded-lg px-3 py-1.5 text-left text-[12.5px] transition-colors disabled:opacity-40",
              item.danger ? "text-destructive hover:bg-destructive/12" : "text-foreground hover:bg-secondary",
            )}
          >
            {item.label}
          </button>
        </React.Fragment>
      ))}
    </div>
  );
};

/** State for a right-click menu: where it is and what is in it. */
export const useContextMenu = () => {
  const [menu, setMenu] = React.useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const open = React.useCallback((e: React.MouseEvent, items: MenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY, items });
  }, []);
  const close = React.useCallback(() => setMenu(null), []);
  const element = menu ? <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={close} /> : null;
  return { open, element };
};
