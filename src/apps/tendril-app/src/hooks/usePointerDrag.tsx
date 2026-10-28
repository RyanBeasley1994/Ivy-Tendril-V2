import React from "react";
import { createPortal } from "react-dom";

/**
 * Drag-and-drop on pointer events, for moving things between lists (projects into sidebar groups,
 * tickets between board lanes).
 *
 * Not HTML5 drag-and-drop: the desktop window has Tauri's native file drop on (`dragDropEnabled`),
 * which takes the webview's drag events for itself, so an HTML5 drag starts but never drops. WebKit
 * also refuses to drag a `<button>` reliably. Pointer events have neither problem.
 *
 * A drag starts after the pointer moves a few pixels, so a plain click still clicks. Targets are
 * any element carrying `data-drop-target="<id>"` (spread {@link PointerDrag.target}); the one under the
 * pointer on release receives the item.
 */
export interface PointerDrag<T> {
  /** Spread onto a draggable element. */
  source: (item: T, label: string) => {
    onPointerDown: (e: React.PointerEvent) => void;
    onClickCapture: (e: React.MouseEvent) => void;
  };
  /** Spread onto a drop target. */
  target: (id: string) => { "data-drop-target": string };
  /** The target the pointer is over during a drag, for highlighting. */
  over: string | null;
  /** The item being dragged, if any. */
  dragging: T | null;
  /** The floating label that follows the pointer; render it once anywhere. */
  ghost: React.ReactNode;
}

const THRESHOLD_PX = 5;

const targetAt = (x: number, y: number): string | null =>
  document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-target]")?.dataset.dropTarget ?? null;

export function usePointerDrag<T>(onDrop: (item: T, target: string) => void): PointerDrag<T> {
  const [drag, setDrag] = React.useState<{ item: T; label: string; x: number; y: number } | null>(null);
  const [over, setOver] = React.useState<string | null>(null);
  const onDropRef = React.useRef(onDrop);
  onDropRef.current = onDrop;
  const suppressClick = React.useRef(false);

  const source = React.useCallback(
    (item: T, label: string) => ({
      onPointerDown: (e: React.PointerEvent) => {
        if (e.button !== 0) return;
        const start = { x: e.clientX, y: e.clientY };
        let active = false;
        suppressClick.current = false;
        const move = (ev: PointerEvent) => {
          if (!active && Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < THRESHOLD_PX) return;
          if (!active) {
            active = true;
            suppressClick.current = true;
            document.body.style.userSelect = "none";
          }
          setDrag({ item, label, x: ev.clientX, y: ev.clientY });
          setOver(targetAt(ev.clientX, ev.clientY));
        };
        const up = (ev: PointerEvent) => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
          window.removeEventListener("pointercancel", up);
          document.body.style.userSelect = "";
          if (active) {
            const t = targetAt(ev.clientX, ev.clientY);
            if (t) onDropRef.current(item, t);
          }
          setDrag(null);
          setOver(null);
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
        window.addEventListener("pointercancel", up);
      },
      // The release that ends a drag also fires a click on the source; that click is not one.
      onClickCapture: (e: React.MouseEvent) => {
        if (suppressClick.current) {
          e.preventDefault();
          e.stopPropagation();
          suppressClick.current = false;
        }
      },
    }),
    [],
  );

  const target = React.useCallback((id: string) => ({ "data-drop-target": id }), []);

  const ghost = drag
    ? createPortal(
        <div
          className="pointer-events-none fixed z-[100] max-w-[220px] -translate-y-1/2 truncate rounded-md border border-primary/50 bg-popover px-2.5 py-1 text-[12px] text-foreground shadow-[0_8px_24px_rgba(0,0,0,0.5)]"
          style={{ left: drag.x + 12, top: drag.y }}
        >
          {drag.label}
        </div>,
        document.body,
      )
    : null;

  return { source, target, over, dragging: drag?.item ?? null, ghost };
}
