import React from "react";
import { formatNumber } from "@/i18n/uiShell";
import type { DashboardFlowStageDto } from "./types.ts";

interface PlanFlowProps {
  stages: DashboardFlowStageDto[];
  /** Plans that fell out of the flow, drawn as a dashed branch off the stage before review. */
  failed?: DashboardFlowStageDto | null;
  onSelect: (stageId: string) => void;
}

const W = 700;
const H = 150;
const MID = 58;
const MIN_HALF = 2.5;
const MAX_HALF = 20;

/**
 * The plan lifecycle as one ribbon: a column per stage, and a band through them whose thickness at
 * each stage is the number of plans sitting there. Nothing is inferred - no pass-through rates, no
 * paths - so the picture says exactly what the counts say: where the work is piling up.
 *
 * The stage names and counts are real buttons laid on a grid with the same column centres as the SVG,
 * so the chart is keyboard-reachable and the SVG itself stays decorative.
 */
export const PlanFlow: React.FC<PlanFlowProps> = ({ stages, failed, onSelect }) => {
  const n = stages.length;
  if (n === 0) return null;

  const max = Math.max(1, ...stages.map((s) => s.count));
  const xs = stages.map((_, i) => ((i + 0.5) * W) / n);
  // Square-root scale: a month of merged plans would otherwise flatten every live stage to a hairline.
  const half = stages.map((s) => MIN_HALF + (MAX_HALF - MIN_HALF) * Math.sqrt(s.count / max));

  const edge = (sign: 1 | -1, order: number[]) =>
    order
      .map((i, k) => {
        const x = xs[i];
        const y = MID + sign * half[i];
        if (k === 0) return `M${x.toFixed(1)},${y.toFixed(1)}`;
        const prev = order[k - 1];
        const mx = (xs[prev] + x) / 2;
        const py = MID + sign * half[prev];
        return `C${mx.toFixed(1)},${py.toFixed(1)} ${mx.toFixed(1)},${y.toFixed(1)} ${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");

  const forward = stages.map((_, i) => i);
  const top = edge(-1, forward);
  const bottom = edge(1, [...forward].reverse()).replace(/^M/, "L");
  const band = `${top} ${bottom} Z`;

  // The failure branch leaves from the stage before review: the last place a plan can fail.
  const branchFrom = Math.max(0, stages.findIndex((s) => s.id === "review") - 1);
  const bx = xs[branchFrom];
  const showFailed = failed != null && failed.count > 0;

  return (
    <div className="tdb-flow">
      <div
        className="tdb-flow-stages"
        style={{ gridTemplateColumns: `repeat(${n}, minmax(0, 1fr))` }}
      >
        {stages.map((stage) => (
          <button
            key={stage.id}
            type="button"
            className="tdb-flow-stage"
            data-active={stage.count > 0}
            onClick={() => onSelect(stage.id)}
          >
            <span className="tdb-flow-stage-label">{stage.label}</span>
            <span className="tdb-flow-stage-count">{formatNumber(stage.count)}</span>
          </button>
        ))}
      </div>
      <svg
        className="tdb-flow-svg"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <defs>
          <linearGradient id="tdb-flow-band" x1="0" x2="1">
            <stop offset="0" stopColor="var(--primary)" stopOpacity="0.35" />
            <stop offset="1" stopColor="var(--primary)" stopOpacity="0.85" />
          </linearGradient>
        </defs>
        {xs.map((x) => (
          <rect
            key={x}
            className="tdb-flow-pillar"
            x={x - 7}
            y={MID - 44}
            width={14}
            height={88}
            rx={7}
          />
        ))}
        <path d={band} fill="url(#tdb-flow-band)" />
        {showFailed && (
          <path
            className="tdb-flow-branch"
            d={`M${bx},${MID + half[branchFrom]} C${bx + 30},${H - 30} ${bx + 60},${H - 18} ${bx + 90},${H - 16}`}
          />
        )}
        {xs.map((x, i) => (
          <circle
            key={x}
            className="tdb-flow-node"
            data-active={stages[i].count > 0}
            cx={x}
            cy={MID}
            r={4.5}
          />
        ))}
      </svg>
      {showFailed && (
        <button
          type="button"
          className="tdb-flow-failed"
          style={{ left: `calc(${(((bx + 90) / W) * 100).toFixed(2)}% + 6px)` }}
          onClick={() => onSelect(failed.id)}
        >
          {failed.label} <span className="tdb-flow-stage-count">{formatNumber(failed.count)}</span>
        </button>
      )}
    </div>
  );
};
