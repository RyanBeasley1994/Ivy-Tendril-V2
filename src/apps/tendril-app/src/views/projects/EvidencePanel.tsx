import React from "react";
import { ChevronLeft, ChevronRight, Film, ImageOff, X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import type { EvidenceGroup, EvidenceItem } from "../../types/projectAssets";
import { bridge } from "../../api/bridge";
import { useAttachmentPreview } from "../../hooks/useAttachmentPreview";
import { describeBridgeError } from "../../types/api";
import { formatAge } from "../../utils/commandCenter";
import { Card, Kbd, Pill } from "../../components/page/kit";

interface FlatItem extends EvidenceItem {
  group: string;
}

const when = (iso?: string) =>
  iso ? `${formatAge(Math.max(0, Date.now() - new Date(iso).getTime()))} ago` : "";

/** What the mission's workers recorded to prove their changes work, by milestone. */
export const EvidencePanel: React.FC<{ missionId: string; running: boolean }> = ({ missionId, running }) => {
  const [groups, setGroups] = React.useState<EvidenceGroup[] | null>(null);
  const [open, setOpen] = React.useState<number | null>(null);

  React.useEffect(() => {
    let live = true;
    const load = () =>
      bridge
        .getMissionEvidence(missionId)
        .then((e) => live && setGroups(e.groups))
        .catch(() => live && setGroups((g) => g ?? []));
    void load();
    // Evidence arrives while the mission runs; once it stops there is nothing new to fetch.
    const timer = running ? window.setInterval(load, 15_000) : undefined;
    return () => {
      live = false;
      if (timer) window.clearInterval(timer);
    };
  }, [missionId, running]);

  const flat = React.useMemo<FlatItem[]>(
    () => (groups ?? []).flatMap((g) => g.items.map((i) => ({ ...i, group: g.title }))),
    [groups],
  );

  if (groups === null) return null;

  return (
    <Card title="Evidence" meta={<Pill tone={flat.length ? "ok" : "mute"}>{flat.length}</Pill>} bodyClassName="gap-4 p-4">
      {flat.length === 0 ? (
        <p className="m-0 text-[12.5px] text-muted-foreground">
          {running
            ? "Nothing recorded yet. Workers attach screenshots and recordings here when they change something you can see."
            : "No screenshots or recordings were attached to this mission."}
        </p>
      ) : (
        groups.map((g) => (
          <section key={`${g.milestoneId ?? "final"}-${g.planFolder}`} className="flex flex-col gap-2">
            <div className="font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">{g.title}</div>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-3">
              {g.items.map((item) => {
                const index = flat.findIndex((f) => f.path === item.path);
                return <Tile key={item.path} item={item} onOpen={() => setOpen(index)} />;
              })}
            </div>
          </section>
        ))
      )}
      {open !== null && flat[open] && (
        <Viewer
          items={flat}
          index={open}
          onIndex={setOpen}
          onClose={() => setOpen(null)}
        />
      )}
    </Card>
  );
};

const Tile: React.FC<{ item: EvidenceItem; onOpen: () => void }> = ({ item, onOpen }) => (
  <button
    type="button"
    onClick={onOpen}
    className="group flex flex-col gap-1.5 text-left"
    aria-label={item.caption || item.file}
  >
    <div className="relative aspect-video overflow-hidden rounded-lg border border-border bg-background transition-colors group-hover:border-primary/50">
      {item.kind === "image" ? <Thumb path={item.path} /> : (
        <div className="flex size-full flex-col items-center justify-center gap-1.5 bg-gradient-to-br from-secondary to-background text-muted-foreground">
          <Film className="size-6" aria-hidden="true" />
          <span className="font-mono text-[10.5px] uppercase tracking-wide">Recording</span>
        </div>
      )}
    </div>
    <span className="line-clamp-2 text-[12px] leading-snug text-foreground">{item.caption || item.file}</span>
    {item.step && <span className="truncate font-mono text-[10.5px] text-muted-foreground">{item.step}</span>}
  </button>
);

const Thumb: React.FC<{ path: string }> = ({ path }) => {
  const { url, failed } = useAttachmentPreview(path, true);
  if (failed) {
    return (
      <div className="flex size-full items-center justify-center text-muted-foreground">
        <ImageOff className="size-5" aria-hidden="true" />
      </div>
    );
  }
  if (!url) return <div className="size-full animate-pulse bg-secondary" />;
  return <img src={url} alt="" className="size-full object-cover object-top" />;
};

/** The full-size view: an image, or a recording loaded only now so a page of clips does not all download. */
const Viewer: React.FC<{
  items: FlatItem[];
  index: number;
  onIndex: (i: number) => void;
  onClose: () => void;
}> = ({ items, index, onIndex, onClose }) => {
  const item = items[index];
  const [video, setVideo] = React.useState<{ url: string | null; error: string | null }>({ url: null, error: null });
  const image = useAttachmentPreview(item.path, item.kind === "image");

  React.useEffect(() => {
    setVideo({ url: null, error: null });
    if (item.kind !== "video") return;
    let live = true;
    bridge
      .getLocalFilePreview(item.path)
      .then((url) => live && setVideo({ url, error: null }))
      .catch((e: unknown) => live && setVideo({ url: null, error: describeBridgeError(e) }));
    return () => {
      live = false;
    };
  }, [item.path, item.kind]);

  const go = React.useCallback(
    (delta: number) => onIndex((index + delta + items.length) % items.length),
    [index, items.length, onIndex],
  );

  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowRight") go(1);
      else if (e.key === "ArrowLeft") go(-1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, onClose]);

  const url = item.kind === "image" ? image.url : video.url;
  const error = item.kind === "image" ? (image.failed ? image.error : null) : video.error;

  return (
    <div
      className="fixed inset-0 z-50 flex flex-col bg-black/85 backdrop-blur-[3px]"
      role="dialog"
      aria-modal="true"
      aria-label="Evidence"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="flex shrink-0 items-center gap-3 px-5 py-3 text-[12.5px] text-muted-foreground">
        <span className="font-mono text-[11px]">{index + 1} / {items.length}</span>
        <span className="truncate">{item.group}</span>
        {item.at && <span className="hidden font-mono text-[11px] sm:inline">{when(item.at)}</span>}
        <span className="ml-auto hidden items-center gap-1.5 sm:flex"><Kbd>←</Kbd><Kbd>→</Kbd> browse <Kbd>esc</Kbd> close</span>
        <button type="button" aria-label="Close" onClick={onClose} className="rounded-lg p-1.5 hover:bg-secondary hover:text-foreground">
          <X className="size-4" aria-hidden="true" />
        </button>
      </div>

      <div className="relative flex min-h-0 flex-1 items-center justify-center px-14" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
        {items.length > 1 && (
          <>
            <button type="button" aria-label="Previous" onClick={() => go(-1)} className="absolute left-3 rounded-full bg-secondary/80 p-2 text-foreground hover:bg-secondary">
              <ChevronLeft className="size-5" aria-hidden="true" />
            </button>
            <button type="button" aria-label="Next" onClick={() => go(1)} className="absolute right-3 rounded-full bg-secondary/80 p-2 text-foreground hover:bg-secondary">
              <ChevronRight className="size-5" aria-hidden="true" />
            </button>
          </>
        )}
        {error ? (
          <div className="max-w-md text-center text-[13px] text-muted-foreground">
            <ImageOff className="mx-auto mb-2 size-6" aria-hidden="true" />
            Could not load this file: {error}
          </div>
        ) : !url ? (
          <div className="text-[13px] text-muted-foreground">{item.kind === "video" ? "Loading the recording…" : "Loading…"}</div>
        ) : item.kind === "image" ? (
          <img src={url} alt={item.caption} className="max-h-full max-w-full rounded-lg object-contain" />
        ) : (
          <video src={url} controls autoPlay className={cn("max-h-full max-w-full rounded-lg bg-black")} />
        )}
      </div>

      <div className="shrink-0 px-6 py-4 text-center">
        {item.caption && <div className="text-[14px] text-foreground">{item.caption}</div>}
        {item.step && <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">{item.step}</div>}
      </div>
    </div>
  );
};
