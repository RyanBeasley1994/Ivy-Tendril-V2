import React from "react";
import { cn } from "@ivy-interactive/components/ui";

/**
 * The V2 command-center primitives: hairline cards with a 44px header, tone pills, key hints, agent
 * avatars and inline mini charts. `Page.tsx` holds the page frame; these are what goes inside it.
 */

export type Tone = "ok" | "warn" | "bad" | "info" | "mute" | "violet";

const PILL_TONES: Record<Tone, string> = {
  ok: "bg-primary/12 text-success",
  warn: "bg-warning/13 text-warning",
  bad: "bg-destructive/14 text-destructive",
  info: "bg-info/14 text-info",
  mute: "bg-secondary text-muted-foreground",
  violet: "bg-violet/14 text-violet",
};

export const Pill: React.FC<{
  tone?: Tone;
  dot?: boolean;
  live?: boolean;
  className?: string;
  children: React.ReactNode;
}> = ({ tone = "mute", dot, live, className, children }) => (
  <span
    className={cn(
      "inline-flex h-5 shrink-0 items-center gap-[5px] whitespace-nowrap rounded-full px-2 text-[11px] font-medium",
      PILL_TONES[tone],
      className,
    )}
  >
    {dot && <Dot live={live} />}
    {children}
  </span>
);

export const Dot: React.FC<{ live?: boolean; className?: string }> = ({ live, className }) => (
  <span
    aria-hidden="true"
    className={cn(
      "inline-block size-1.5 shrink-0 rounded-full bg-current",
      live && "shadow-[0_0_0_3px_color-mix(in_srgb,currentColor_22%,transparent)]",
      className,
    )}
  />
);

export const Kbd: React.FC<{ children: React.ReactNode; className?: string }> = ({
  children,
  className,
}) => (
  <kbd
    className={cn(
      "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[5px] border border-input bg-muted px-[5px] font-mono text-[10.5px] text-muted-foreground",
      className,
    )}
  >
    {children}
  </kbd>
);

/** Uppercase mono section label. */
export const Label: React.FC<{ children: React.ReactNode; className?: string }> = ({
  children,
  className,
}) => (
  <span
    className={cn(
      "font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground",
      className,
    )}
  >
    {children}
  </span>
);

/**
 * A hairline card: 44px header with a title and trailing actions, then its body. `flush` bodies
 * (tables, lists with their own row borders) get no padding.
 */
export const Card: React.FC<{
  title?: React.ReactNode;
  meta?: React.ReactNode;
  actions?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
  bodyClassName?: string;
  testId?: string;
  as?: "section" | "div";
}> = ({ title, meta, actions, children, className, bodyClassName, testId, as = "section" }) => {
  const Tag = as;
  return (
    <Tag
      data-testid={testId}
      className={cn(
        "flex min-w-0 flex-col overflow-hidden rounded-xl border border-border bg-card shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]",
        className,
      )}
    >
      {(title || actions) && (
        <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border/70 px-4">
          {title && <h2 className="m-0 truncate text-[13px] font-semibold text-foreground">{title}</h2>}
          {meta}
          {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
        </div>
      )}
      <div className={cn("flex min-h-0 flex-1 flex-col", bodyClassName)}>{children}</div>
    </Tag>
  );
};

/** Compact segmented control for card headers (24px buttons). */
export function Seg<T extends string>({
  value,
  options,
  onChange,
  label,
  className,
}: {
  value: T;
  options: { value: T; label: React.ReactNode }[];
  onChange: (value: T) => void;
  label: string;
  className?: string;
}) {
  return (
    <div
      role="tablist"
      aria-label={label}
      className={cn("flex gap-0.5 rounded-lg border border-border bg-background p-0.5", className)}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="tab"
          aria-selected={value === option.value}
          onClick={() => onChange(option.value)}
          className={cn(
            "inline-flex h-6 items-center gap-1.5 rounded-md px-2.5 text-[11.5px] transition-colors",
            value === option.value
              ? "bg-secondary text-foreground shadow-[inset_0_0_0_1px_var(--input)]"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export const GhostButton: React.FC<
  React.ButtonHTMLAttributes<HTMLButtonElement> & { size?: "sm" | "md" }
> = ({ className, size = "md", ...props }) => (
  <button
    type="button"
    {...props}
    className={cn(
      "inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg border border-input bg-muted px-[11px] text-[12.5px] text-foreground transition-colors hover:bg-secondary disabled:opacity-50",
      size === "sm" ? "h-7" : "h-[30px]",
      className,
    )}
  />
);

export const PrimaryButton: React.FC<
  React.ButtonHTMLAttributes<HTMLButtonElement> & { size?: "sm" | "md" | "lg" }
> = ({ className, size = "md", ...props }) => (
  <button
    type="button"
    {...props}
    className={cn(
      "inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg bg-primary px-3 text-[12.5px] font-semibold text-primary-foreground transition-colors hover:bg-success disabled:opacity-50",
      size === "sm" ? "h-7" : size === "lg" ? "h-[38px] text-[13px]" : "h-[30px]",
      className,
    )}
  />
);

/** Kbd inside a primary button: tinted to sit on the green. */
export const PrimaryKbd: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <kbd className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[5px] border border-primary-foreground/25 bg-primary-foreground/15 px-[5px] font-mono text-[10.5px] text-primary-foreground">
    {children}
  </kbd>
);

export const IconButton: React.FC<
  React.ButtonHTMLAttributes<HTMLButtonElement> & { label: string }
> = ({ className, label, ...props }) => (
  <button
    type="button"
    aria-label={label}
    title={label}
    {...props}
    className={cn(
      "inline-flex size-[30px] shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground",
      className,
    )}
  />
);

/** A thin progress bar. */
export const Bar: React.FC<{ value: number; className?: string; fill?: string; height?: number }> = ({
  value,
  className,
  fill = "bg-primary",
  height = 4,
}) => (
  <span
    className={cn("block overflow-hidden rounded-full bg-secondary", className)}
    style={{ height }}
  >
    <span
      className={cn("block h-full rounded-full", fill)}
      style={{ width: `${Math.max(0, Math.min(100, value))}%` }}
    />
  </span>
);

/** Agent identity from a model or provider string: claude, codex/gpt, gemini, copilot, other. */
export type AgentKind = "claude" | "codex" | "gemini" | "copilot" | "other";

export const agentKind = (name: string | undefined | null): AgentKind => {
  const n = (name ?? "").toLowerCase();
  if (n.includes("claude") || n.includes("opus") || n.includes("sonnet") || n.includes("haiku"))
    return "claude";
  if (n.includes("codex") || n.includes("gpt") || n.includes("openai") || /\bo\d/.test(n))
    return "codex";
  if (n.includes("gemini")) return "gemini";
  if (n.includes("copilot")) return "copilot";
  return "other";
};

const AGENT_STYLE: Record<AgentKind, { cls: string; letter: string }> = {
  claude: { cls: "bg-primary/14 text-success", letter: "C" },
  codex: { cls: "bg-info/16 text-info", letter: "O" },
  gemini: { cls: "bg-violet/16 text-violet", letter: "G" },
  copilot: { cls: "bg-warning/15 text-warning", letter: "P" },
  other: { cls: "bg-secondary text-foreground", letter: "·" },
};

/** Bar/line colour per agent, for charts. */
export const AGENT_FILL: Record<AgentKind, string> = {
  claude: "var(--success)",
  codex: "var(--info)",
  gemini: "var(--violet)",
  copilot: "var(--warning)",
  other: "var(--muted-foreground)",
};

export const AgentAvatar: React.FC<{
  agent: string | undefined | null;
  size?: number;
  className?: string;
  letter?: string;
}> = ({ agent, size = 22, className, letter }) => {
  const kind = agentKind(agent);
  const style = AGENT_STYLE[kind];
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-flex shrink-0 items-center justify-center font-semibold",
        style.cls,
        className,
      )}
      style={{
        width: size,
        height: size,
        borderRadius: Math.round(size / 3.2),
        fontSize: Math.max(8.5, size * 0.46),
      }}
    >
      {letter ?? (kind === "other" && agent ? agent.charAt(0).toUpperCase() : style.letter)}
    </span>
  );
};

/** Tiny vertical bars; the last bar is full strength, the rest faded. */
export const MiniBars: React.FC<{
  values: number[];
  color?: string;
  height?: number;
  barWidth?: number;
  gap?: number;
}> = ({ values, color = "var(--success)", height = 22, barWidth = 4, gap = 2 }) => {
  const max = Math.max(1, ...values);
  return (
    <span aria-hidden="true" className="inline-flex shrink-0 items-end" style={{ height, gap }}>
      {values.map((v, i) => (
        <span
          key={i}
          style={{
            width: barWidth,
            height: Math.max(2, Math.round((v / max) * height)),
            background: color,
            opacity: i === values.length - 1 ? 1 : 0.45,
            borderRadius: 2,
          }}
        />
      ))}
    </span>
  );
};

/** A small area sparkline. */
export const Spark: React.FC<{
  values: number[];
  width?: number;
  height?: number;
  color?: string;
}> = ({ values, width = 80, height = 26, color = "var(--success)" }) => {
  if (values.length < 2) return null;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const range = max - min || 1;
  const pts = values.map((v, i) => {
    const x = (i * width) / (values.length - 1);
    const y = height - 3 - ((v - min) / range) * (height - 6);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} fill="none" aria-hidden="true">
      <polygon points={`0,${height} ${pts.join(" ")} ${width},${height}`} fill={color} opacity={0.12} />
      <polyline
        points={pts.join(" ")}
        stroke={color}
        strokeWidth={1.6}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
};

/** Heatmap ramp, empty to hottest. */
export const HEAT = ["#121920", "#0f3b2f", "#0a7a58", "#12b886", "#3fe0ae"];

export const heatLevel = (value: number, max: number): number => {
  if (value <= 0 || max <= 0) return 0;
  const r = value / max;
  return r < 0.15 ? 1 : r < 0.4 ? 2 : r < 0.7 ? 3 : 4;
};

const isMac = () => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

/** The platform's modifier glyph: ⌘ on macOS, Ctrl elsewhere. */
export const modKey = (): string => (isMac() ? "⌘" : "Ctrl");
