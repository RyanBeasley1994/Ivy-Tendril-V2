import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import { bridge } from "../../api/bridge";
import { describeBridgeError } from "../../types/api";
import type { DiffTarget, FileDiff } from "../../types/git";

export interface DiffLine {
  kind: "add" | "del" | "ctx" | "hunk" | "note";
  text: string;
  oldNo?: number;
  newNo?: number;
}

/** Splits git's unified diff into numbered lines, and keeps the header facts (new file, rename, mode). */
export function parseDiff(text: string): { facts: string[]; lines: DiffLine[] } {
  const facts: string[] = [];
  const lines: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  const rawLines = text.split("\n");
  // The text ends with a newline, which would otherwise read as one more (empty) line.
  if (rawLines[rawLines.length - 1] === "") rawLines.pop();
  for (const raw of rawLines) {
    if (raw.startsWith("@@")) {
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw);
      oldNo = m ? Number(m[1]) : 0;
      newNo = m ? Number(m[2]) : 0;
      inHunk = true;
      lines.push({ kind: "hunk", text: raw });
    } else if (!inHunk) {
      if (/^(new file|deleted file|rename |copy |old mode|new mode|similarity)/.test(raw)) facts.push(raw);
      else if (raw.startsWith("Binary files")) facts.push(raw);
    } else if (raw.startsWith("+")) {
      lines.push({ kind: "add", text: raw.slice(1), newNo: newNo++ });
    } else if (raw.startsWith("-")) {
      lines.push({ kind: "del", text: raw.slice(1), oldNo: oldNo++ });
    } else if (raw.startsWith("\\")) {
      lines.push({ kind: "note", text: raw.slice(1).trim() });
    } else {
      // A context line starts with a space; an empty one inside a hunk is a blank context line.
      lines.push({ kind: "ctx", text: raw.slice(1), oldNo: oldNo++, newNo: newNo++ });
    }
  }
  return { facts, lines };
}

const MAX_RENDERED_LINES = 4000;

/** A parsed diff, drawn: gutters with both line numbers, the sign, and the text. */
export const DiffBody: React.FC<{ diff: FileDiff }> = ({ diff }) => {
  const parsed = React.useMemo(() => parseDiff(diff.text), [diff.text]);
  const [all, setAll] = React.useState(false);
  const shown = all ? parsed.lines : parsed.lines.slice(0, MAX_RENDERED_LINES);

  if (diff.binary) {
    return <div className="px-4 py-6 text-[12.5px] text-muted-foreground">Binary file: there is no text to compare.</div>;
  }
  if (parsed.lines.length === 0) {
    return (
      <div className="px-4 py-6 text-[12.5px] text-muted-foreground">
        {parsed.facts.length > 0 ? parsed.facts.join(" · ") : "No textual changes (a mode or whitespace-only change, or nothing left to show)."}
      </div>
    );
  }
  return (
    <div className="overflow-x-auto">
      {parsed.facts.length > 0 && <div className="border-b border-border/60 px-4 py-1.5 font-mono text-[11px] text-muted-foreground">{parsed.facts.join(" · ")}</div>}
      <table className="w-full border-collapse font-mono text-[12px] leading-[18px]">
        <tbody>
          {shown.map((l, i) =>
            l.kind === "hunk" ? (
              <tr key={i} className="bg-info/8 text-info">
                <td colSpan={4} className="px-3 py-0.5">{l.text}</td>
              </tr>
            ) : l.kind === "note" ? (
              <tr key={i} className="text-muted-foreground">
                <td colSpan={4} className="px-3 py-0.5 italic">{l.text}</td>
              </tr>
            ) : (
              <tr key={i} className={cn(l.kind === "add" && "bg-primary/10", l.kind === "del" && "bg-destructive/10")}>
                <td className="w-[44px] select-none px-2 text-right text-muted-foreground/60">{l.oldNo ?? ""}</td>
                <td className="w-[44px] select-none px-2 text-right text-muted-foreground/60">{l.newNo ?? ""}</td>
                <td className={cn("w-4 select-none text-center", l.kind === "add" ? "text-success" : l.kind === "del" ? "text-destructive" : "text-muted-foreground/40")}>
                  {l.kind === "add" ? "+" : l.kind === "del" ? "−" : ""}
                </td>
                <td className="whitespace-pre pr-4 text-foreground">{l.text}</td>
              </tr>
            ),
          )}
        </tbody>
      </table>
      {(parsed.lines.length > shown.length || diff.truncated) && (
        <div className="flex items-center gap-3 border-t border-border/60 px-4 py-2 text-[12px] text-muted-foreground">
          {diff.truncated && <span>This diff is very large, so only the start is shown.</span>}
          {parsed.lines.length > shown.length && (
            <button type="button" className="text-info hover:underline" onClick={() => setAll(true)}>
              Show all {parsed.lines.length} lines
            </button>
          )}
        </div>
      )}
    </div>
  );
};

/** Loads and shows one file's diff. */
export const DiffPane: React.FC<{
  repoId: string;
  target: DiffTarget;
  path: string;
  origPath?: string;
  /** Changes whenever the repository does, so an open diff does not go stale. */
  version?: unknown;
}> = ({ repoId, target, path, origPath, version }) => {
  const [diff, setDiff] = React.useState<FileDiff | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const key = JSON.stringify([repoId, target, path, origPath]);

  React.useEffect(() => {
    let live = true;
    setError(null);
    bridge
      .gitDiff(repoId, target, path, origPath)
      .then((d) => live && setDiff(d))
      .catch((e) => live && setError(describeBridgeError(e)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, version]);

  // The previous file's diff stays up while the next loads, so the pane does not flash empty.
  if (error) return <div className="px-4 py-6 text-[12.5px] text-destructive">{error}</div>;
  if (!diff || diff.path !== path) return <div className="px-4 py-6 text-[12.5px] text-muted-foreground">Loading…</div>;
  return <DiffBody diff={diff} />;
};
