import React from "react";
import { formatCompact, formatDate, useTranslation } from "@/i18n/uiShell";
import { rampLevel, type DashboardTokenDayDto, type DashboardTokenShareDto } from "./types.ts";

interface TokenHeatmapProps {
  days: DashboardTokenDayDto[];
  share?: DashboardTokenShareDto[];
}

const DAY_MS = 86_400_000;

/** `yyyy-MM-dd` as a UTC day number, so gaps can be filled without time-zone drift. */
const dayNumber = (iso: string): number | null => {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!match) return null;
  return Math.floor(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])) / DAY_MS);
};

const isoOf = (day: number): string => new Date(day * DAY_MS).toISOString().slice(0, 10);

const compact = (value: number): string =>
  formatCompact(value, { maximumFractionDigits: value >= 1_000_000 ? 2 : 0 });

/**
 * A contribution-graph of tokens per day: one dot per day, a column per week, Monday at the top.
 * The input may be sparse; days it skips are drawn as empty. The row labels and the legend make the
 * scale readable, and the whole grid carries one accessible summary rather than a name per dot.
 */
export const TokenHeatmap: React.FC<TokenHeatmapProps> = ({ days, share = [] }) => {
  const { t } = useTranslation("uiShell");

  const parsed = days
    .map((day) => ({ day: dayNumber(day.date), tokens: day.tokens }))
    .filter((entry): entry is { day: number; tokens: number } => entry.day != null);

  if (parsed.length === 0) {
    return <div className="tdb-empty-note">{t("dashboard.tokens.empty")}</div>;
  }

  const byDay = new Map(parsed.map((entry) => [entry.day, entry.tokens]));
  const first = Math.min(...parsed.map((entry) => entry.day));
  const last = Math.max(...parsed.map((entry) => entry.day));
  const range = Array.from({ length: last - first + 1 }, (_, index) => first + index);
  const values = range.map((day) => byDay.get(day) ?? 0);

  const total = values.reduce((sum, value) => sum + value, 0);
  const max = Math.max(...values);
  const activeDays = values.filter((value) => value > 0);
  const average = activeDays.length ? total / activeDays.length : 0;
  const today = values[values.length - 1] ?? 0;
  const weeks = Math.max(1, Math.ceil(range.length / 7));

  // Day 0 of the Unix epoch was a Thursday; shift so Monday is row 0.
  const leading = (first + 3) % 7;
  const cells: ({ day: number; value: number } | null)[] = [
    ...Array.from({ length: leading }, () => null),
    ...range.map((day, index) => ({ day, value: values[index] })),
  ];

  const shareTotal = share.reduce((sum, slice) => sum + slice.tokens, 0);

  return (
    <div className="tdb-tokens-body">
      <div className="tdb-tokens-figures">
        <span className="tdb-tokens-total">{compact(total)}</span>
        <span className="tdb-tokens-window">
          {t("dashboard.tokens.window", { weeks: String(weeks) })}
        </span>
        <span className="tdb-tokens-stats">
          {compact(today)} {t("dashboard.tokens.today")} ·{" "}
          {t("dashboard.tokens.average", { value: compact(average) })} ·{" "}
          {t("dashboard.tokens.peak", { value: compact(max) })}
        </span>
      </div>

      <div className="tdb-heat">
        <div className="tdb-heat-days" aria-hidden="true">
          {[0, 1, 2, 3, 4, 5, 6].map((row) => (
            <span key={row}>
              {/* Mon, Wed and Fri: 2024-01-01 was a Monday, so row n is that date plus n days. */}
              {row % 2 === 1 || row === 6
                ? ""
                : formatDate(new Date(Date.UTC(2024, 0, 1 + row)), {
                    weekday: "narrow",
                    timeZone: "UTC",
                  })}
            </span>
          ))}
        </div>
        <div
          className="tdb-heat-grid"
          role="img"
          aria-label={`${t("dashboard.tokens.title")}: ${compact(total)}, ${t(
            "dashboard.tokens.window",
            { weeks: String(weeks) },
          )}`}
        >
          {cells.map((cell, index) =>
            cell == null ? (
              <span key={`pad-${index}`} className="tdb-heat-cell" data-pad="true" />
            ) : (
              <span
                key={cell.day}
                className="tdb-heat-cell"
                data-level={rampLevel(cell.value, max)}
                title={t("dashboard.tokens.cell", {
                  date: formatDate(new Date(isoOf(cell.day) + "T00:00:00Z"), {
                    month: "short",
                    day: "numeric",
                    timeZone: "UTC",
                  }),
                  value: compact(cell.value),
                })}
              />
            ),
          )}
        </div>
      </div>

      <div className="tdb-heat-legend" aria-hidden="true">
        <span>{t("dashboard.tokens.less")}</span>
        {[0, 1, 2, 3, 4].map((level) => (
          <span key={level} className="tdb-heat-cell" data-level={level} />
        ))}
        <span>{t("dashboard.tokens.more")}</span>
      </div>

      {shareTotal > 0 && (
        <div className="tdb-share">
          <div className="tdb-share-bar" aria-hidden="true">
            {share.map((slice, index) => (
              <span
                key={slice.label}
                data-tone={index % 5}
                style={{ flexGrow: slice.tokens / shareTotal }}
              />
            ))}
          </div>
          <ul className="tdb-share-legend" aria-label={t("dashboard.tokens.share")}>
            {share.map((slice, index) => (
              <li key={slice.label}>
                <span className="tdb-share-swatch" data-tone={index % 5} />
                {slice.label}
                <span className="tdb-share-pct">
                  {Math.round((slice.tokens / shareTotal) * 100)}%
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
};
