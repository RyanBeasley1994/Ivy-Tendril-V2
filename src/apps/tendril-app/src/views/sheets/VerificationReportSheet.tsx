import React, { useEffect, useState } from "react";
import { Button } from "@ivy-interactive/components/ui";
import { useTranslation } from "../../i18n";
import { VerificationReportSheet as VerificationReportSheetView } from "@ivy-interactive/components/dialogs";
import { bridge } from "../../api/bridge";
import type { VerificationReport, VerificationStatus } from "../../types/api";
import { describeBridgeError } from "../../types/api";

export interface VerificationReportSheetProps {
  /** The plan id whose verification report is being inspected. */
  planId: string | null | undefined;
  /** The verification name (e.g. "CheckResult", "DotnetTest"), or null if closed. */
  verificationName: string | null;
  /** Initial status for the badge while the report is loading or if frontmatter has no result. */
  initialStatus?: VerificationStatus;
  onClose: () => void;
  wireframeBaseUrl?: string;
  /**
   * Records the outcome by hand. Given, the sheet offers Pass / Fail / Skip, for a check the
   * operator ran themselves or one an agent ran but never recorded.
   */
  onSetStatus?: (status: VerificationStatus) => Promise<void>;
}

/**
 * The connected half of V1's `VerificationReportSheet` (`Apps/Views/Sheets/VerificationReportSheet.cs`):
 * reads `<planFolder>/Verification/<name>.md` over the bridge and hands the read's state to the
 * library sheet, which owns everything visible.
 */
export const VerificationReportSheet: React.FC<VerificationReportSheetProps> = ({
  planId,
  verificationName,
  initialStatus,
  onClose,
  wireframeBaseUrl,
  onSetStatus,
}) => {
  const { t } = useTranslation("review");
  const [saving, setSaving] = useState<VerificationStatus | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [report, setReport] = useState<VerificationReport | null>(null);
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!planId || !verificationName) {
      setReport(null);
      setError(null);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);
    setReport(null);

    bridge
      .getVerificationReport(planId, verificationName)
      .then((res) => {
        if (!cancelled) {
          setReport(res);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(describeBridgeError(err));
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [planId, verificationName]);

  const record = async (status: VerificationStatus) => {
    if (!onSetStatus) return;
    setSaving(status);
    setSaveError(null);
    try {
      await onSetStatus(status);
    } catch (err) {
      setSaveError(describeBridgeError(err));
    } finally {
      setSaving(null);
    }
  };

  const statusActions = onSetStatus ? (
    <div className="flex flex-col gap-2 rounded-lg border border-border/70 bg-muted/30 p-3">
      <p className="m-0 text-xs text-muted-foreground">{t("verifications.recordHint")}</p>
      <div className="flex flex-wrap gap-2">
        {(["Pass", "Fail", "Skipped"] as const).map((status) => (
          <Button
            key={status}
            type="button"
            size="sm"
            variant={status === "Pass" ? "default" : "outline"}
            disabled={saving !== null || initialStatus === status}
            onClick={() => void record(status)}
            data-testid={`verification-record-${status}`}
          >
            {t(`verifications.record.${status}`)}
          </Button>
        ))}
      </div>
      {saveError && <p className="m-0 text-xs text-destructive">{saveError}</p>}
    </div>
  ) : undefined;

  return (
    <VerificationReportSheetView
      statusActions={statusActions}
      verificationName={verificationName}
      onClose={onClose}
      report={report}
      loading={loading}
      error={error}
      initialStatus={initialStatus}
      wireframeBaseUrl={wireframeBaseUrl}
    />
  );
};
