import React from "react";
import { cn, Skeleton } from "@ivy-interactive/components/ui";

/**
 * The command-center page kit. Every screen is built from the same four pieces - a padded page, a
 * header with the title and its actions, a strip of headline figures, and panels - so the layout
 * language is one thing across the app rather than one per view.
 */

export const Page: React.FC<{
  children: React.ReactNode;
  className?: string;
  /** Pages that own their own scroll (a table with a sticky header) pass false. */
  scroll?: boolean;
  testId?: string;
}> = ({ children, className, scroll = true, testId }) => (
  <div
    data-testid={testId}
    className={cn(
      "flex min-h-0 flex-1 flex-col gap-4 px-5 pb-6 pt-4",
      scroll ? "overflow-y-auto" : "overflow-hidden",
      className,
    )}
  >
    {children}
  </div>
);

export const PageHeader: React.FC<{
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
}> = ({ title, subtitle, actions }) => (
  <header className="flex shrink-0 flex-wrap items-end gap-3">
    <div className="flex min-w-0 flex-col gap-1.5">
      <h1 className="m-0 text-[22px] font-semibold leading-tight tracking-[-0.02em] text-foreground">
        {title}
      </h1>
      {subtitle && <p className="m-0 text-[12.5px] text-muted-foreground">{subtitle}</p>}
    </div>
    {actions && <div className="ml-auto flex flex-wrap items-center gap-2">{actions}</div>}
  </header>
);

export const Panel: React.FC<{
  title?: React.ReactNode;
  actions?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  bodyClassName?: string;
  testId?: string;
}> = ({ title, actions, children, className, bodyClassName, testId }) => (
  <section
    data-testid={testId}
    className={cn(
      "flex min-w-0 flex-col rounded-[14px] border border-border bg-card text-card-foreground",
      className,
    )}
  >
    {(title || actions) && (
      <div className="flex min-h-[52px] shrink-0 items-center gap-2 px-5 pt-4 pb-2">
        {title && <h2 className="m-0 text-sm font-semibold text-foreground">{title}</h2>}
        {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
      </div>
    )}
    <div className={cn("flex min-h-0 flex-1 flex-col px-5 pb-4", bodyClassName)}>{children}</div>
  </section>
);

export interface StatItem {
  id: string;
  label: string;
  value: string;
  /** A small pill under the value, e.g. "↗ 12%". */
  note?: string | null;
  /** A muted line under the value saying what it is based on. */
  hint?: string | null;
  tone?: "neutral" | "good" | "bad" | "info";
  onSelect?: () => void;
}

const NOTE_TONES: Record<NonNullable<StatItem["tone"]>, string> = {
  neutral: "bg-secondary text-muted-foreground",
  good: "bg-success/12 text-success",
  bad: "bg-destructive/12 text-destructive",
  info: "bg-info/14 text-info",
};

/**
 * The strip before its figures exist: the same cells, blocked out, so nothing moves when they land and
 * no cell ever states a dash for a number that simply has not arrived.
 */
export const StatStripSkeleton: React.FC<{ count: number; label: string; testId?: string }> = ({
  count,
  label,
  testId,
}) => (
  <div
    data-testid={testId}
    role="status"
    aria-label={label}
    className="grid shrink-0 overflow-hidden rounded-[14px] border border-border bg-card"
    style={{ gridTemplateColumns: `repeat(${count}, minmax(0, 1fr))` }}
  >
    {Array.from({ length: count }, (_, index) => (
      <div
        key={index}
        className={cn("flex flex-col gap-3 px-[18px] py-4", index > 0 && "border-l border-border")}
      >
        <Skeleton className="h-3 w-3/5" />
        <Skeleton className="h-6 w-2/5" />
      </div>
    ))}
  </div>
);

/** A row of headline figures in one panel, divided by hairlines, the Insights summary strip. */
export const StatStrip: React.FC<{ items: StatItem[]; testId?: string }> = ({ items, testId }) => (
  <div
    data-testid={testId}
    className="grid shrink-0 overflow-hidden rounded-[14px] border border-border bg-card"
    style={{ gridTemplateColumns: `repeat(${Math.max(1, items.length)}, minmax(0, 1fr))` }}
  >
    {items.map((item, index) => {
      const body = (
        <>
          <span className="text-xs text-muted-foreground">{item.label}</span>
          <span
            data-stat-value=""
            className="text-xl font-semibold leading-none tracking-[-0.02em] text-foreground"
          >
            {item.value}
          </span>
          {item.note && (
            <span
              className={cn(
                "self-start rounded-full px-2 py-0.5 text-[11px] font-medium",
                NOTE_TONES[item.tone ?? "neutral"],
              )}
            >
              {item.note}
            </span>
          )}
          {item.hint && <span className="text-[11.5px] leading-snug text-muted-foreground">{item.hint}</span>}
        </>
      );
      const cls = cn(
        "flex min-w-0 flex-col gap-2 px-[18px] py-4 text-left",
        index > 0 && "border-l border-border",
      );
      return item.onSelect ? (
        <button
          key={item.id}
          type="button"
          data-stat-clickable="true"
          onClick={item.onSelect}
          className={cn(cls, "transition-colors hover:bg-secondary/60 focus-visible:outline-2 focus-visible:outline-ring")}
        >
          {body}
        </button>
      ) : (
        <div key={item.id} className={cls}>
          {body}
        </div>
      );
    })}
  </div>
);

/** Segmented control for a panel header or a page header: 7D / 30D, Cost / Plans. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div
      role="tablist"
      aria-label={label}
      className="flex gap-0.5 rounded-[9px] border border-border bg-background/60 p-[3px]"
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="tab"
          aria-selected={value === option.value}
          onClick={() => onChange(option.value)}
          className={cn(
            "h-7 rounded-[7px] px-3 text-[12.5px] transition-colors",
            value === option.value
              ? "bg-secondary text-foreground"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
