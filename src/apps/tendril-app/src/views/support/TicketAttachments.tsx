import React from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, FileText, ImageOff, Loader2, X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { supportApi, type TicketAttachmentData } from "../../api/support";
import { isTauri } from "../../utils/tauri";

type Attachment = { url: string; filename: string; type?: string };

const IMAGE_RE = /\.(png|jpe?g|gif|webp|bmp|svg|heic)$/i;
const isImage = (a: Attachment) => Boolean(a.type?.startsWith("image/")) || IMAGE_RE.test(a.filename);

/** Fetched once per attachment while the app is open: switching tickets back and forth costs nothing. */
const cache = new Map<string, Promise<TicketAttachmentData>>();
const load = (ticketId: string, messageId: string, index: number) => {
  const key = `${ticketId}/${messageId}/${index}`;
  let hit = cache.get(key);
  if (!hit) {
    hit = supportApi.attachment(ticketId, messageId, index);
    hit.catch(() => cache.delete(key));
    cache.set(key, hit);
  }
  return hit;
};

const dataUrl = (d: TicketAttachmentData) => `data:${d.contentType || "application/octet-stream"};base64,${d.dataBase64}`;

const sizeText = (bytes: number) =>
  bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;

/** Saves an attachment where the operator picks (the desktop app), or downloads it (a browser). */
async function saveAttachment(d: TicketAttachmentData): Promise<void> {
  if (isTauri()) {
    const path = await save({ defaultPath: d.filename });
    if (!path) return;
    await invoke("cmd_save_file", { path, dataBase64: d.dataBase64 });
    return;
  }
  const a = document.createElement("a");
  a.href = dataUrl(d);
  a.download = d.filename;
  a.click();
}

const ImageAttachment: React.FC<{ ticketId: string; messageId: string; index: number; a: Attachment; onOpen: (d: TicketAttachmentData) => void }> = ({
  ticketId,
  messageId,
  index,
  a,
  onOpen,
}) => {
  const [data, setData] = React.useState<TicketAttachmentData | null>(null);
  const [failed, setFailed] = React.useState<string | null>(null);
  React.useEffect(() => {
    let live = true;
    load(ticketId, messageId, index)
      .then((d) => live && setData(d))
      .catch((e: unknown) => live && setFailed(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [ticketId, messageId, index]);

  if (failed) {
    return (
      <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground" title={failed}>
        <ImageOff className="size-3.5" aria-hidden="true" />
        {a.filename}: could not load
      </span>
    );
  }
  if (!data) {
    return (
      <span className="flex h-20 w-32 items-center justify-center rounded-md border border-border bg-muted/30 text-muted-foreground">
        <Loader2 className="size-4 animate-spin" aria-label={`Loading ${a.filename}`} />
      </span>
    );
  }
  return (
    <button type="button" onClick={() => onOpen(data)} className="block cursor-zoom-in" title={`${a.filename} (click to enlarge)`}>
      <img src={dataUrl(data)} alt={a.filename} className="max-h-56 max-w-full rounded-md border border-border object-contain" />
    </button>
  );
};

const FileAttachment: React.FC<{ ticketId: string; messageId: string; index: number; a: Attachment }> = ({ ticketId, messageId, index, a }) => {
  const [busy, setBusy] = React.useState(false);
  const [problem, setProblem] = React.useState<string | null>(null);
  const download = async () => {
    setBusy(true);
    setProblem(null);
    try {
      await saveAttachment(await load(ticketId, messageId, index));
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <button
      type="button"
      onClick={() => void download()}
      disabled={busy}
      title={problem ?? `Save ${a.filename}`}
      className={cn(
        "flex max-w-full items-center gap-1.5 self-start rounded-md border border-border bg-muted/30 px-2 py-1 text-[11.5px] text-foreground hover:bg-muted/60",
        problem && "border-destructive/40 text-destructive",
      )}
    >
      <FileText className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="truncate">{a.filename}</span>
      {busy ? (
        <Loader2 className="size-3.5 shrink-0 animate-spin" aria-hidden="true" />
      ) : (
        <Download className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      )}
    </button>
  );
};

/** A message's attachments: images shown (click for full size), other files saved with a click. */
export const TicketAttachments: React.FC<{ ticketId: string; messageId: string; attachments: Attachment[] }> = ({
  ticketId,
  messageId,
  attachments,
}) => {
  const [open, setOpen] = React.useState<TicketAttachmentData | null>(null);
  React.useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const shown = attachments.map((a, index) => ({ a, index })).filter(({ a }) => a.url);
  if (shown.length === 0) return null;
  return (
    <div className="mt-1 flex flex-wrap items-start gap-2">
      {shown.map(({ a, index }) =>
        isImage(a) ? (
          <ImageAttachment key={index} ticketId={ticketId} messageId={messageId} index={index} a={a} onOpen={setOpen} />
        ) : (
          <FileAttachment key={index} ticketId={ticketId} messageId={messageId} index={index} a={a} />
        ),
      )}
      {open && (
        <div
          role="dialog"
          aria-label={open.filename}
          className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-3 bg-black/80 p-6"
          onClick={() => setOpen(null)}
        >
          <img src={dataUrl(open)} alt={open.filename} className="max-h-[85vh] max-w-[92vw] rounded-md object-contain" />
          <div className="flex items-center gap-3 text-[12px] text-white/80" onClick={(e) => e.stopPropagation()}>
            <span className="truncate">{open.filename} · {sizeText(open.size)}</span>
            <button type="button" onClick={() => void saveAttachment(open)} className="flex items-center gap-1 rounded px-2 py-1 hover:bg-white/10">
              <Download className="size-3.5" aria-hidden="true" /> Save
            </button>
            <button type="button" onClick={() => setOpen(null)} className="flex items-center gap-1 rounded px-2 py-1 hover:bg-white/10" aria-label="Close">
              <X className="size-3.5" aria-hidden="true" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
