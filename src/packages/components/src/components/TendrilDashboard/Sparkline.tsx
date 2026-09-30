import React from "react";

interface SparklineProps {
  values: number[];
  width?: number;
  height?: number;
  className?: string;
}

/**
 * A trend line with a faint area under it, in `currentColor`, so the caller's tone class colours it.
 * Decorative: the figure it sits beside is the accessible value, so the SVG is hidden from the tree.
 * Fewer than two points draw nothing rather than a misleading flat line.
 */
export const Sparkline: React.FC<SparklineProps> = ({
  values,
  width = 84,
  height = 28,
  className,
}) => {
  if (values.length < 2) return null;

  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const pad = 3;
  const points = values.map((value, index) => {
    const x = (index / (values.length - 1)) * width;
    const y = height - pad - ((value - min) / span) * (height - pad * 2);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const line = points.join(" ");

  return (
    <svg
      className={className}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      fill="none"
      aria-hidden="true"
    >
      <polygon
        points={`0,${height} ${line} ${width},${height}`}
        fill="currentColor"
        opacity={0.12}
      />
      <polyline
        points={line}
        stroke="currentColor"
        strokeWidth={1.6}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
};
