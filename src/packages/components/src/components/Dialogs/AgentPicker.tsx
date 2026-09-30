import * as React from "react";
import { Check, ChevronDown, Users } from "lucide-react";
import { cn } from "@/lib/utils";
import { useOutsideClick } from "../../hooks/use-outside-click";

/**
 * The Create Plan dialog's harness pickers as one compact chip in the composer's action row, opening
 * a small panel above it. The panel is rendered in place rather than portalled, so it lives inside
 * the dialog's focus scope and a click in it is never an "outside" click that closes the dialog.
 */

export interface AgentPickerOption {
  value: string;
  label: string;
}

/** Tint per agent family, so the same agent reads the same everywhere. */
const agentTint = (label: string): string => {
  const n = label.toLowerCase();
  if (/claude|opus|sonnet|haiku/.test(n)) return "bg-primary/15 text-success";
  if (/codex|gpt|openai/.test(n)) return "bg-info/16 text-info";
  if (/gemini/.test(n)) return "bg-violet/16 text-violet";
  if (/copilot/.test(n)) return "bg-warning/15 text-warning";
  return "bg-secondary text-foreground";
};

const AgentMark: React.FC<{ label: string; size?: number }> = ({ label, size = 16 }) => {
  const letter = label.replace(/^default\s*\(?/i, "").trim().charAt(0).toUpperCase() || "·";
  return (
    <span
      aria-hidden="true"
      className={cn("inline-flex shrink-0 items-center justify-center font-semibold", agentTint(label))}
      style={{ width: size, height: size, borderRadius: Math.round(size / 3.2), fontSize: size * 0.55 }}
    >
      {letter}
    </span>
  );
};

const chipClass =
  "inline-flex h-8 min-w-0 max-w-[200px] items-center gap-1.5 rounded-full border border-border bg-card/60 pl-1.5 pr-2 text-[12.5px] text-foreground transition-colors hover:bg-secondary aria-expanded:bg-secondary";

/** "Default (Claude)" reads as "Claude" on the chip; the panel keeps the full label. */
const shortName = (label: string): string => label.replace(/^[^(]*\((.+)\)\s*$/, "$1");

function usePanel() {
  const [open, setOpen] = React.useState(false);
  const wrapRef = React.useRef<HTMLDivElement>(null);
  const close = React.useCallback(() => setOpen(false), []);
  useOutsideClick(open, [wrapRef], close);
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && open) {
      // Close the panel, not the dialog behind it.
      e.stopPropagation();
      e.preventDefault();
      setOpen(false);
    }
  };
  return { open, setOpen, wrapRef, onKeyDown };
}

const panelClass =
  "absolute bottom-full left-0 z-50 mb-2 rounded-xl border border-input bg-popover p-1.5 text-popover-foreground shadow-[0_20px_50px_-12px_rgba(0,0,0,0.8)]";

/** One harness for a plan: a chip that opens a checked list. */
export const AgentPicker: React.FC<{
  label: string;
  value: string;
  defaultLabel: string;
  options: AgentPickerOption[];
  onChange: (value: string) => void;
  /** Which way the panel opens. The composer opens it upward; a settings row, downward. */
  side?: "top" | "bottom";
  disabled?: boolean;
}> = ({ label, value, defaultLabel, options, onChange, side = "top", disabled = false }) => {
  const { open, setOpen, wrapRef, onKeyDown } = usePanel();
  const all = [{ value: "", label: defaultLabel }, ...options];
  const current = all.find((o) => o.value === value) ?? all[0];

  return (
    <div ref={wrapRef} className="relative min-w-0" onKeyDown={onKeyDown} data-testid="harness-pickers">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={`${label}: ${current.label}`}
        title={label}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className={cn(chipClass, "disabled:cursor-not-allowed disabled:opacity-50")}
      >
        <AgentMark label={current.label} />
        <span className="truncate">{shortName(current.label)}</span>
        <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      </button>
      {open && (
        <div
          role="listbox"
          aria-label={label}
          className={cn(
            panelClass,
            "w-60",
            side === "bottom" && "bottom-auto top-full mb-0 mt-2",
            side === "bottom" && "left-auto right-0",
          )}
        >
          <div className="px-2.5 pb-1 pt-1.5 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
            {label}
          </div>
          {all.map((o) => {
            const selected = o.value === value;
            return (
              <button
                key={o.value || "default"}
                type="button"
                role="option"
                aria-selected={selected}
                onClick={() => {
                  onChange(o.value);
                  setOpen(false);
                }}
                className={cn(
                  "flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] transition-colors hover:bg-secondary",
                  selected && "bg-secondary/60",
                )}
              >
                <AgentMark label={o.label} size={18} />
                <span className="truncate">{o.label}</span>
                {selected && <Check className="ml-auto size-3.5 text-success" aria-hidden="true" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
};

/** One harness per mission role: a single chip summarising them, opening a row of pills per role. */
export function RoleAgentsPicker<R extends string>({
  label,
  roles,
  roleLabel,
  values,
  defaultLabel,
  options,
  onChange,
  defaultShortLabel,
  mixedLabel,
}: {
  label: string;
  /** "Default", for the pill that leaves a role on the configured agent. */
  defaultShortLabel: string;
  /** The chip's summary when the roles use different agents, e.g. "3 agents". */
  mixedLabel: (count: number) => string;
  roles: readonly R[];
  roleLabel: (role: R) => string;
  values: Record<R, string>;
  defaultLabel: string;
  options: AgentPickerOption[];
  onChange: (role: R, value: string) => void;
}) {
  const { open, setOpen, wrapRef, onKeyDown } = usePanel();
  const all = [{ value: "", label: defaultLabel }, ...options];
  const labelOf = (v: string) => all.find((o) => o.value === v)?.label ?? defaultLabel;
  const distinct = Array.from(new Set(roles.map((r) => values[r])));
  const summary =
    distinct.length === 1 ? `${shortName(labelOf(distinct[0]))} ×${roles.length}` : mixedLabel(distinct.length);

  return (
    <div ref={wrapRef} className="relative min-w-0" onKeyDown={onKeyDown} data-testid="harness-pickers">
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`${label}: ${summary}`}
        onClick={() => setOpen((v) => !v)}
        className={chipClass}
      >
        <span className="flex -space-x-1">
          {distinct.slice(0, 3).map((v) => (
            <span key={v || "default"} className="rounded-[6px] ring-2 ring-card">
              <AgentMark label={labelOf(v)} />
            </span>
          ))}
        </span>
        <span className="truncate">{summary}</span>
        <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      </button>
      {open && (
        <div role="dialog" aria-label={label} className={cn(panelClass, "w-[360px] p-2")}>
          <div className="flex items-center gap-1.5 px-1.5 pb-2 pt-1 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground">
            <Users className="size-3" aria-hidden="true" />
            {label}
          </div>
          <div className="flex flex-col gap-1">
            {roles.map((role) => (
              <div
                key={role}
                role="radiogroup"
                aria-label={roleLabel(role)}
                className="flex items-center gap-2 rounded-lg px-1.5 py-1"
              >
                <span className="w-[72px] shrink-0 text-xs text-muted-foreground">{roleLabel(role)}</span>
                <div className="flex flex-wrap gap-1">
                  {all.map((o) => {
                    const selected = values[role] === o.value;
                    return (
                      <button
                        key={o.value || "default"}
                        type="button"
                        role="radio"
                        aria-checked={selected}
                        title={o.label}
                        onClick={() => onChange(role, o.value)}
                        className={cn(
                          "inline-flex h-7 items-center gap-1.5 rounded-full border px-2 text-xs transition-colors",
                          selected
                            ? "border-primary/40 bg-primary/12 text-foreground"
                            : "border-transparent text-muted-foreground hover:bg-secondary hover:text-foreground",
                        )}
                      >
                        <AgentMark label={o.label} size={14} />
                        {o.value === "" ? defaultShortLabel : o.label}
                      </button>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
