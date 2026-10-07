import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import type { Graph, GraphRef, GraphRow } from "../../types/git";
import { ago, laneColor } from "./gitUi";

const ROW = 32;
const LANE = 16;
const PAD = 14;
const MAX_LANES = 14;
const OVERSCAN = 12;

const x = (lane: number) => PAD + Math.min(lane, MAX_LANES - 1) * LANE;

/** One row's slice of the graph: the lines that cross it and its dot. */
const RowGraph: React.FC<{ row: GraphRow; width: number; isHead: boolean }> = ({ row, width, isHead }) => {
  const mid = ROW / 2;
  return (
    <svg width={width} height={ROW} className="shrink-0" aria-hidden="true">
      {row.lines.map((l, i) => {
        const stroke = laneColor(l.color);
        const x1 = x(l.from);
        const x2 = x(l.to);
        const d =
          l.part === "full"
            ? `M ${x1} 0 L ${x1} ${ROW}`
            : l.part === "top"
              ? `M ${x1} 0 C ${x1} ${mid * 0.55}, ${x2} ${mid * 0.45}, ${x2} ${mid}`
              : `M ${x1} ${mid} C ${x1} ${mid + mid * 0.55}, ${x2} ${mid + mid * 0.45}, ${x2} ${ROW}`;
        return <path key={i} d={d} stroke={stroke} strokeWidth={2} fill="none" strokeLinecap="round" />;
      })}
      <circle
        cx={x(row.lane)}
        cy={mid}
        r={isHead ? 6 : row.parents.length > 1 ? 5 : 4.5}
        fill={row.parents.length > 1 ? "var(--card)" : laneColor(row.color)}
        stroke={laneColor(row.color)}
        strokeWidth={row.parents.length > 1 || isHead ? 2.5 : 0}
      />
    </svg>
  );
};

const REF_STYLE: Record<GraphRef["kind"], string> = {
  head: "bg-primary text-primary-foreground",
  local: "bg-primary/15 text-success",
  remote: "bg-info/15 text-info",
  tag: "bg-warning/15 text-warning",
};

/** At most this many labels sit before a subject; the rest fold into "+N" so the message stays readable. */
const MAX_CHIPS = 2;

const RefChips: React.FC<{ refs: GraphRef[] }> = ({ refs }) => {
  // HEAD is folded into the branch it names; then local branches, remote branches, tags.
  const order = { head: 0, local: 1, remote: 2, tag: 3 } as const;
  const named = refs.some((r) => r.kind === "local" && r.isHead) ? refs.filter((r) => r.kind !== "head") : refs;
  const sorted = [...named].sort((a, b) => order[a.kind] - order[b.kind]);
  const shown = sorted.slice(0, MAX_CHIPS);
  const hidden = sorted.slice(MAX_CHIPS);
  return (
    <span className="flex min-w-0 max-w-[55%] shrink-0 items-center gap-1 overflow-hidden">
      {shown.map((r) => (
        <span
          key={`${r.kind}-${r.name}`}
          className={cn(
            "rounded px-1.5 py-px font-mono text-[10.5px] leading-[16px]",
            REF_STYLE[r.kind],
            // The branch you are on is always readable in full; the rest give way when the row is tight.
            r.isHead && r.kind === "local" ? "shrink-0 ring-1 ring-primary/60" : "min-w-0 max-w-[170px] truncate",
          )}
          title={`${r.kind === "tag" ? "Tag" : r.kind === "remote" ? "Remote branch" : r.kind === "head" ? "HEAD" : "Branch"}: ${r.name}`}
        >
          {r.name}
        </span>
      ))}
      {hidden.length > 0 && (
        <span className="shrink-0 rounded bg-secondary px-1.5 py-px font-mono text-[10.5px] leading-[16px] text-muted-foreground" title={hidden.map((h) => h.name).join(", ")}>
          +{hidden.length}
        </span>
      )}
    </span>
  );
};

/** The commit list with its graph. Only the rows near the viewport are in the DOM. */
export const CommitGraph: React.FC<{
  graph: Graph;
  selected: string | null;
  wip: { count: number; conflicted: number } | null;
  onSelect: (hash: string | "wip") => void;
  onContext: (row: GraphRow, e: React.MouseEvent) => void;
  onLoadMore: () => void;
}> = ({ graph, selected, wip, onSelect, onContext, onLoadMore }) => {
  const scroller = React.useRef<HTMLDivElement>(null);
  const [view, setView] = React.useState({ top: 0, height: 600 });

  React.useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const measure = () => setView({ top: el.scrollTop, height: el.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const graphWidth = PAD * 2 + Math.min(Math.max(graph.width, 1), MAX_LANES) * LANE - LANE / 2;
  const headHash = graph.rows.find((r) => r.refs.some((ref) => ref.kind === "head"))?.hash;
  const first = Math.max(0, Math.floor(view.top / ROW) - OVERSCAN);
  const last = Math.min(graph.rows.length, Math.ceil((view.top + view.height) / ROW) + OVERSCAN);

  // Keep a selected commit in view when it changes from outside (a branch clicked in the sidebar).
  React.useEffect(() => {
    if (!selected || selected === "wip") return;
    const i = graph.rows.findIndex((r) => r.hash === selected);
    const el = scroller.current;
    if (i < 0 || !el) return;
    const top = i * ROW;
    if (top < el.scrollTop || top + ROW > el.scrollTop + el.clientHeight) el.scrollTo({ top: Math.max(0, top - el.clientHeight / 3) });
  }, [selected, graph.rows]);

  return (
    <div ref={scroller} onScroll={(e) => setView({ top: e.currentTarget.scrollTop, height: e.currentTarget.clientHeight })} className="@container min-h-0 flex-1 overflow-y-auto" data-testid="commit-graph">
      {wip && (
        <button
          type="button"
          onClick={() => onSelect("wip")}
          className={cn("flex h-[34px] w-full items-center gap-3 border-b border-border/60 px-2 text-left text-[12.5px]", selected === "wip" ? "bg-secondary" : "hover:bg-secondary/50")}
        >
          <svg width={graphWidth} height={ROW} className="shrink-0" aria-hidden="true">
            <circle cx={x(0)} cy={ROW / 2} r={5} fill="none" stroke={wip.conflicted ? "var(--destructive)" : "var(--warning)"} strokeWidth={2} strokeDasharray="3 3" />
          </svg>
          <span className={cn("font-medium", wip.conflicted ? "text-destructive" : "text-warning")}>
            {wip.conflicted ? `${wip.conflicted} conflicted` : `${wip.count} uncommitted change${wip.count === 1 ? "" : "s"}`}
          </span>
          <span className="text-muted-foreground">Working changes</span>
        </button>
      )}

      <div style={{ height: graph.rows.length * ROW, position: "relative" }}>
        {graph.rows.slice(first, last).map((row, k) => {
          const i = first + k;
          return (
            <div
              key={row.hash}
              role="button"
              tabIndex={0}
              onClick={() => onSelect(row.hash)}
              onKeyDown={(e) => e.key === "Enter" && onSelect(row.hash)}
              onContextMenu={(e) => onContext(row, e)}
              style={{ position: "absolute", top: i * ROW, left: 0, right: 0, height: ROW }}
              className={cn("flex cursor-pointer items-center gap-3 px-2 text-[12.5px]", selected === row.hash ? "bg-secondary" : "hover:bg-secondary/50")}
            >
              <RowGraph row={row} width={graphWidth} isHead={row.hash === headHash} />
              {row.refs.length > 0 && <RefChips refs={row.refs} />}
              <span className={cn("min-w-0 flex-1 truncate", row.hash === headHash ? "font-medium text-foreground" : "text-foreground/90")}>{row.subject}</span>
              <span className="hidden w-[110px] shrink-0 truncate text-right text-[11.5px] text-muted-foreground @[640px]:block">{row.author}</span>
              <span className="hidden w-[40px] shrink-0 text-right font-mono text-[10.5px] text-muted-foreground @[420px]:block">{ago(row.at).replace(" ago", "")}</span>
              <span className="hidden w-[56px] shrink-0 font-mono text-[10.5px] text-muted-foreground @[520px]:block">{row.short}</span>
            </div>
          );
        })}
      </div>

      {graph.more && (
        <div className="flex justify-center py-3">
          <button type="button" onClick={onLoadMore} className="rounded-lg border border-input bg-muted px-3 py-1.5 text-[12px] text-foreground hover:bg-secondary">
            Load older commits
          </button>
        </div>
      )}
      {graph.rows.length === 0 && <div className="px-6 py-10 text-center text-[13px] text-muted-foreground">No commits yet.</div>}
    </div>
  );
};
