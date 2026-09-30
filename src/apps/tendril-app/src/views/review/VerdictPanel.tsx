import React from "react";
import { Check, Circle, LoaderCircle, Minus, X } from "lucide-react";
import { cn } from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import type { Job, PlanVerification } from "../../types/api";
import { Label, Pill } from "../../components/page/kit";
import { formatTokens } from "../../utils/commandCenter";
import { formatCurrency } from "../../utils/dashboardMetrics";
import { formatDuration } from "../../utils/fleet";

const STATUS_ICON: Record<PlanVerification["status"], React.ReactNode> = {
  Pass: <Check className="size-[13px] text-success" strokeWidth={2.6} />,
  Fail: <X className="size-[13px] text-destructive" strokeWidth={2.6} />,
  Pending: <LoaderCircle className="size-[13px] animate-spin text-info" />,
  Skipped: <Minus className="size-[13px] text-muted-foreground" />,
};

/**
 * Review's verdict column: how many of the plan's checks pass, each check (click for its report),
 * and what the plan cost to produce. Every figure is the plan's own; nothing here is estimated.
 */
export const VerdictPanel: React.FC<{
  verifications: PlanVerification[];
  jobs: Job[];
  onOpenReport: (name: string) => void;
}> = ({ verifications, jobs, onOpenReport }) => {
  const { t } = useTranslation("review");
  const counted = verifications.filter((v) => v.status !== "Skipped");
  const passed = counted.filter((v) => v.status === "Pass").length;
  const failed = counted.filter((v) => v.status === "Fail").length;
  const pending = counted.filter((v) => v.status === "Pending").length;
  const ratio = counted.length ? passed / counted.length : 0;

  const tokens = jobs.reduce((s, j) => s + (j.tokens ?? 0), 0);
  const cost = jobs.reduce((s, j) => s + (j.cost ?? 0), 0);
  const seconds = jobs.reduce((s, j) => {
    if (j.durationSeconds != null) return s + j.durationSeconds;
    const a = Date.parse(j.startedAt ?? "");
    const b = Date.parse(j.completedAt ?? "");
    return Number.isNaN(a) || Number.isNaN(b) ? s : s + (b - a) / 1000;
  }, 0);

  const verdict =
    counted.length === 0
      ? { tone: "mute" as const, label: t("verdict.noChecks") }
      : failed > 0
        ? { tone: "bad" as const, label: t("verdict.failing", { count: failed }) }
        : pending > 0
          ? { tone: "info" as const, label: t("verdict.running") }
          : { tone: "ok" as const, label: t("verdict.ready") };

  const r = 34;
  const c = 2 * Math.PI * r;
  const ringColor = failed > 0 ? "var(--destructive)" : pending > 0 ? "var(--info)" : "var(--primary)";

  return (
    <div data-testid="review-verdict" className="flex flex-col gap-4 px-4 py-4">
      <div className="flex items-center gap-2">
        <h2 className="m-0 text-sm font-semibold">{t("verdict.title")}</h2>
        <Pill tone={verdict.tone} dot className="ml-auto">
          {verdict.label}
        </Pill>
      </div>

      <div className="flex items-center gap-4">
        <svg width={86} height={86} viewBox="0 0 86 86" aria-hidden="true" className="shrink-0">
          <circle cx={43} cy={43} r={r} stroke="var(--secondary)" strokeWidth={7} fill="none" />
          {counted.length > 0 && (
            <circle
              cx={43}
              cy={43}
              r={r}
              stroke={ringColor}
              strokeWidth={7}
              fill="none"
              strokeLinecap="round"
              strokeDasharray={`${(c * ratio).toFixed(1)} ${c.toFixed(1)}`}
              transform="rotate(-90 43 43)"
            />
          )}
          <text x={43} y={43} textAnchor="middle" fill="var(--foreground)" fontSize={17} fontWeight={600}>
            {counted.length ? `${passed}/${counted.length}` : "—"}
          </text>
          <text x={43} y={57} textAnchor="middle" fill="var(--muted-foreground)" fontSize={9} fontFamily="var(--font-mono)">
            {t("verdict.checks").toUpperCase()}
          </text>
        </svg>
        <div className="grid flex-1 grid-cols-1 gap-2 text-xs">
          {[
            [t("verdict.tokens"), tokens > 0 ? (formatTokens(tokens) ?? "—") : "—"],
            [t("verdict.cost"), cost > 0 ? formatCurrency(cost) : "—"],
            [t("verdict.time"), seconds > 0 ? formatDuration(seconds) : "—"],
            [t("verdict.runs"), String(jobs.length)],
          ].map(([label, value]) => (
            <div key={label} className="flex items-center">
              <span className="text-muted-foreground">{label}</span>
              <span className="ml-auto font-mono text-[11.5px]">{value}</span>
            </div>
          ))}
        </div>
      </div>

      <div className="flex flex-col">
        <Label className="pb-1.5">{t("verdict.checks")}</Label>
        {verifications.length === 0 && (
          <p data-testid="no-verifications" className="m-0 border-t border-border/70 py-2.5 text-xs text-muted-foreground">
            {t("verifications.none")}
          </p>
        )}
        {verifications.map((v) => (
          <button
            key={v.name}
            type="button"
            data-testid={`review-verification-button-${v.name}`}
            onClick={() => onOpenReport(v.name)}
            title={t("verifications.viewReport", { name: v.name })}
            className={cn(
              "flex h-8 items-center gap-2.5 border-t border-border/70 text-left text-xs transition-colors hover:text-success",
            )}
          >
            <span className="flex w-4 justify-center">{STATUS_ICON[v.status] ?? <Circle className="size-3" />}</span>
            <span className="truncate font-mono text-[11.5px]">{v.name}</span>
            <span className="ml-auto font-mono text-[10.5px] text-muted-foreground">
              {t(`verificationStatus.${v.status.toLowerCase()}` as "verificationStatus.pass")}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
};
