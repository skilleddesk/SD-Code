import { useMemo, useState } from 'react';

import { strings } from '../../strings';
import { analyticsOf, type DayTotal } from '../../store/analytics';
import { useAppStore } from '../../store/store';

/**
 * The Analytics tab - spec section 7.12, rebuilt from the log in v4.
 *
 * Three stat tiles (turns, reported cost, tokens), one bar chart (the last seven days), and the share
 * of turns by engine. Every number is `store/analytics.ts` reading what the engines reported at the end
 * of each turn - the tab used to print a fixed week of spending and a plan limit this build has no way
 * to see, so the "Limits" block is gone rather than guessed.
 *
 * The chart is one series in the accent colour: bars anchored to the baseline with 4px rounded tops, a
 * gap between them, a recessive baseline, a tooltip per bar, and the same numbers as a table below for
 * anyone who does not read charts. It shows cost when any turn reported one, and turns otherwise.
 */
const WIDTH = 320;
const HEIGHT = 120;
const BASE = 100;
const PLOT = 84;

function money(value: number): string {
  return value >= 100 ? `$${value.toFixed(0)}` : value >= 1 ? `$${value.toFixed(2)}` : `$${value.toFixed(3)}`;
}

function tokens(value: number): string {
  return value >= 1_000_000 ? `${(value / 1_000_000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value);
}

function weekday(day: string): string {
  const [year, month, date] = day.split('-').map(Number);

  return new Date(year ?? 1970, (month ?? 1) - 1, date ?? 1).toLocaleDateString(undefined, { weekday: 'short' });
}

function DayChart({ days, byCost }: { days: DayTotal[]; byCost: boolean }) {
  const [hover, setHover] = useState<number | null>(null);
  const value = (day: DayTotal): number => (byCost ? day.cost : day.turns);
  const max = Math.max(...days.map(value), byCost ? 0.01 : 1);
  const slot = WIDTH / days.length;
  const bar = slot - 10;
  const shown = hover === null ? null : days[hover];

  return (
    <div className="relative">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="w-full"
        style={{ height: HEIGHT }}
        role="img"
        aria-label={strings.rightPanel.analytics.chartLabel(byCost)}
        onMouseLeave={() => setHover(null)}
      >
        <line x1="0" x2={WIDTH} y1={BASE + 0.5} y2={BASE + 0.5} stroke="var(--border-default)" strokeWidth="1" />
        <text x="0" y="9" fontSize="9" fill="var(--text-muted)" fontFamily="var(--font-mono)">
          {byCost ? money(max) : String(max)}
        </text>

        {days.map((day, index) => {
          const height = value(day) === 0 ? 0 : Math.max(3, (value(day) / max) * PLOT);
          const x = index * slot + 5;
          const top = BASE - height;

          return (
            <g key={day.day}>
              {height === 0 ? null : (
                /* A rounded top, a square foot: the bar stands on the baseline. */
                <path
                  d={`M${x},${BASE} L${x},${top + 4} Q${x},${top} ${x + 4},${top} L${x + bar - 4},${top} Q${x + bar},${top} ${x + bar},${top + 4} L${x + bar},${BASE} Z`}
                  fill="var(--accent)"
                  opacity={hover === null || hover === index ? 1 : 0.45}
                />
              )}
              <text
                x={x + bar / 2}
                y={HEIGHT - 4}
                fontSize="9"
                textAnchor="middle"
                fill={hover === index ? 'var(--text-primary)' : 'var(--text-muted)'}
                fontFamily="var(--font-ui)"
              >
                {weekday(day.day)}
              </text>
              {/* The hit target is the whole column, taller and wider than the bar. */}
              <rect
                x={index * slot}
                y="0"
                width={slot}
                height={HEIGHT}
                fill="transparent"
                onMouseEnter={() => setHover(index)}
              />
            </g>
          );
        })}
      </svg>

      {shown === null || shown === undefined ? null : (
        <div
          className="pointer-events-none absolute top-[4px] rounded-md border border-border-default bg-bg-overlay px-[8px] py-[5px] text-[11px] shadow-md"
          style={{ left: `${Math.min(70, Math.max(0, ((hover ?? 0) / days.length) * 100 - 5))}%` }}
          role="status"
        >
          <div className="font-medium text-text-primary">{weekday(shown.day)} · {shown.day}</div>
          <div className="font-mono tabular-nums text-text-secondary">
            {strings.rightPanel.analytics.dayLine(shown.turns, byCost ? money(shown.cost) : null)}
          </div>
        </div>
      )}
    </div>
  );
}

function Tile({ label, value, note }: { label: string; value: string; note: string }) {
  return (
    <div className="stat min-w-0 flex-1 rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px]">
      <div className="text-[10px] font-semibold uppercase tracking-[.08em] text-text-muted">{label}</div>
      <div className="mt-[2px] font-mono text-[18px] font-semibold tabular-nums text-text-primary">{value}</div>
      <div className="truncate text-[10.5px] text-text-muted" title={note}>
        {note}
      </div>
    </div>
  );
}

export function AnalyticsTab() {
  const turns = useAppStore((state) => state.turns);
  const analytics = useMemo(() => analyticsOf(turns), [turns]);
  const copy = strings.rightPanel.analytics;
  const byCost = analytics.priced > 0;

  if (analytics.turns === 0) {
    return (
      <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] leading-[1.6] text-text-muted">
        {copy.empty}
      </div>
    );
  }

  return (
    <div className="analytics-wrap flex flex-col gap-[12px] p-[12px]">
      <div className="flex gap-[8px]">
        <Tile label={copy.turnsTitle} value={String(analytics.turns)} note={copy.failedNote(analytics.failed)} />
        <Tile label={copy.costTitle} value={byCost ? money(analytics.cost) : '—'} note={copy.costNote(analytics.priced, analytics.turns)} />
        <Tile label={copy.tokensTitle} value={tokens(analytics.input + analytics.output)} note={copy.tokensNote(tokens(analytics.input), tokens(analytics.output))} />
      </div>

      <section className="chart rounded-md border border-border-subtle bg-bg-raised p-[14px]">
        <div className="chart-title mb-[10px] text-[10.5px] font-bold uppercase tracking-[.08em] text-text-muted">
          {copy.chartTitle(byCost)}
        </div>
        <DayChart days={analytics.days} byCost={byCost} />
        <table className="sr-only">
          <caption>{copy.chartTitle(byCost)}</caption>
          <thead>
            <tr>
              <th>{copy.day}</th>
              <th>{copy.turnsTitle}</th>
              <th>{copy.costTitle}</th>
            </tr>
          </thead>
          <tbody>
            {analytics.days.map((day) => (
              <tr key={day.day}>
                <td>{day.day}</td>
                <td>{day.turns}</td>
                <td>{money(day.cost)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="chart rounded-md border border-border-subtle bg-bg-raised p-[14px]">
        <div className="chart-title mb-[10px] text-[10.5px] font-bold uppercase tracking-[.08em] text-text-muted">{copy.byEngineTitle}</div>
        <div className="bar-list flex flex-col gap-[8px]">
          {analytics.engines.map((row) => (
            <div key={row.engine} className="bar-item flex items-center gap-[10px] font-mono text-[11px] text-text-secondary">
              <span className="bar-label w-[100px] shrink-0 truncate" title={row.engine}>
                {row.engine}
              </span>
              <div className="bar-track h-[6px] flex-1 overflow-hidden rounded-full bg-bg-input">
                <div className="bar-fill h-full rounded-full bg-accent" style={{ width: `${row.percent}%` }} />
              </div>
              <span className="bar-pct w-[64px] text-right text-[10.5px] tabular-nums text-text-muted">
                {copy.engineShare(row.turns, row.percent)}
              </span>
            </div>
          ))}
        </div>
      </section>

      <p className="text-[10.5px] leading-[1.55] text-text-muted">{copy.source}</p>
    </div>
  );
}
