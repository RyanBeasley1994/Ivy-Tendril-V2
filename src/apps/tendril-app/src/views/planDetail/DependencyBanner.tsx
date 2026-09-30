import React from "react";
import { Hourglass } from "lucide-react";
import { Callout } from "@ivy-interactive/components/ui";
import type { PlanDetail, PlanSummary } from "../../types/api";
import { PlanActionsController } from "../../controllers/planActions";
import { planStateLabel } from "../../i18n/enumLabels";
import { useTranslation, type TFunction } from "../../i18n";

/**
 * Why a plan is waiting, said on the plan rather than in a disabled button's tooltip: which plan it
 * builds on, where that plan is, and what happens next. Renders nothing for a plan that waits on
 * nothing, and nothing once the plan has started.
 */
export const DependencyBanner: React.FC<{
  plan: PlanDetail;
  allPlans: PlanSummary[];
  onOpenPlan: (planId: string) => void;
}> = ({ plan, allPlans, onOpenPlan }) => {
  const { t } = useTranslation("plans");
  if (plan.state !== "Draft" && plan.state !== "Blocked") return null;
  const waiting = PlanActionsController.waitingOn(plan, allPlans);
  if (waiting.length === 0) return null;
  const queued = plan.state === "Blocked";

  return (
    <Callout.Warning className="flex-1" data-testid="plan-dependency-banner">
      <div className="flex flex-col gap-1.5">
        <p className="m-0 flex items-center gap-1.5 text-sm font-medium">
          <Hourglass className="size-3.5 shrink-0" aria-hidden="true" />
          {queued ? t("dependency.queuedTitle") : t("dependency.title")}
        </p>
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {waiting.map(({ ref, plan: dep }) => {
            const id = (dep?.id ?? ref).slice(0, 5);
            return (
              <li key={ref} className="flex flex-wrap items-center gap-x-2 text-sm">
                <button
                  type="button"
                  className="font-mono underline underline-offset-2 hover:opacity-80"
                  onClick={() => onOpenPlan(id)}
                >
                  #{id}
                </button>
                <span>{dep?.title ?? ref}</span>
                <span className="opacity-80">
                  · {dep ? planStateLabel(dep.state) : t("dependency.missing")}
                </span>
                {dep && <span className="opacity-80">— {nextStep(dep.state, t)}</span>}
              </li>
            );
          })}
        </ul>
        <p className="m-0 text-xs opacity-80">
          {queued ? t("dependency.queuedHint") : t("dependency.hint")}
        </p>
      </div>
    </Callout.Warning>
  );
};

/** What has to happen to the dependency for this plan to run. */
function nextStep(state: string, t: TFunction<"plans">): string {
  switch (state) {
    case "Review":
    case "Failed":
      return t("dependency.next.createPr");
    case "Draft":
    case "Blocked":
      return t("dependency.next.execute");
    case "Executing":
    case "Creating":
    case "Updating":
      return t("dependency.next.running");
    default:
      return t("dependency.next.complete");
  }
}
