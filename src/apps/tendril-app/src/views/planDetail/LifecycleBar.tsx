import React from "react";
import { cn } from "@ivy-interactive/components/ui";
import { useEnumLabels } from "../../i18n/enumLabels";
import type { PlanLifecycleState } from "../../types/api";

/** The four stages every plan passes through, in order. */
const STAGES: PlanLifecycleState[] = ["Draft", "Executing", "Review", "Completed"];

/** Which stage a state sits in. States outside the lifecycle (Icebox, Skipped) sit in none. */
const STAGE_OF: Partial<Record<PlanLifecycleState, number>> = {
  Creating: 0,
  Updating: 0,
  Draft: 0,
  Blocked: 0,
  Executing: 1,
  Review: 2,
  Failed: 2,
  Completed: 3,
};

/**
 * Where the plan is in its life, as a row of segments under the title: stages passed are filled,
 * the current one is lit, and a failed plan's current segment turns red. Each label is the state's
 * own localized name.
 */
export const LifecycleBar: React.FC<{ state: PlanLifecycleState }> = ({ state }) => {
  const labels = useEnumLabels();
  const current = STAGE_OF[state];
  if (current == null) return null;
  const failed = state === "Failed";

  return (
    <ol
      className="m-0 flex w-full max-w-[64rem] list-none gap-1.5 p-0"
      aria-label={labels.planState(state)}
      data-testid="plan-lifecycle-bar"
    >
      {STAGES.map((stage, index) => {
        const done = index < current;
        const now = index === current;
        return (
          <li
            key={stage}
            aria-current={now ? "step" : undefined}
            className="flex flex-1 flex-col gap-1.5"
          >
            <span
              className={cn(
                "h-1 rounded-full",
                done && "bg-primary/60",
                now && (failed ? "bg-destructive" : "bg-primary"),
                !done && !now && "bg-secondary",
              )}
            />
            <span
              className={cn(
                "text-[11px]",
                now ? "font-medium text-foreground" : done ? "text-muted-foreground" : "text-muted-foreground/70",
              )}
            >
              {labels.planState(stage)}
            </span>
          </li>
        );
      })}
    </ol>
  );
};
