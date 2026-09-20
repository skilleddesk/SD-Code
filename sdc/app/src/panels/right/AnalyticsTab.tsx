import { strings } from '../../strings';

/**
 * The Analytics tab - spec section 7.12.
 *
 * Three cards, in the order the spec lists them: the last seven days as a bar chart, the week's
 * total as one large mono number, then spending by engine as proportion bars and the one limit that
 * ever bites - the Claude Max subscription, at an estimated 60% with two hours left in the window.
 *
 * The chart is hand-written SVG rather than a charting library: it is seven rectangles and a
 * two-stop gradient, and the spec's own numbers (`3 files changed`, `$4.12`, `68%`) are placeholders
 * until the cost ledger of a later step fills them in. `preserveAspectRatio="none"` lets it stretch
 * to whatever width the panel is dragged to.
 *
 * The gradient's stops read `--accent`, so the chart is blue in the dark theme and a different blue
 * in the light one without this file knowing either value.
 */
const CHART_VIEWBOX = '0 0 320 120';
const BAR_GRADIENT_ID = 'sdc-analytics-bars';

function SpendingChart() {
  const bars = strings.rightPanel.analytics.spendBars;

  return (
    <div className="chart mb-[12px] rounded-md border border-border-subtle bg-bg-raised p-[14px]">
      <div className="chart-title mb-[12px] flex items-center justify-between text-[10.5px] font-bold uppercase tracking-[.08em] text-text-muted">
        <span>{strings.rightPanel.analytics.spending}</span>
        <span className="chip inline-flex items-center gap-[5px] rounded-md border border-border-subtle bg-bg-raised px-[8px] py-[3px] font-mono text-[11px] normal-case tracking-normal text-text-secondary">
          {strings.rightPanel.analytics.range}
        </span>
      </div>

      <svg
        viewBox={CHART_VIEWBOX}
        preserveAspectRatio="none"
        className="w-full"
        style={{ height: 120 }}
        role="img"
        aria-label={strings.rightPanel.analytics.spending}
      >
        <defs>
          <linearGradient id={BAR_GRADIENT_ID} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopOpacity="1" style={{ stopColor: 'var(--accent)' }} />
            <stop offset="100%" stopOpacity="0.25" style={{ stopColor: 'var(--accent)' }} />
          </linearGradient>
        </defs>
        {bars.map((value, index) => (
          <rect
            key={index}
            x={12 + index * 46}
            y={100 - value}
            width={index === bars.length - 1 ? 20 : 28}
            height={value}
            rx="3"
            fill={`url(#${BAR_GRADIENT_ID})`}
          />
        ))}
      </svg>
    </div>
  );
}

export function AnalyticsTab() {
  const analytics = strings.rightPanel.analytics;

  return (
    <div className="analytics-wrap p-[12px]">
      <SpendingChart />

      <div className="chart mb-[12px] rounded-md border border-border-subtle bg-bg-raised p-[14px]">
        <div className="chart-title mb-[12px] text-[10.5px] font-bold uppercase tracking-[.08em] text-text-muted">
          {analytics.totalTitle}
        </div>
        <div className="font-mono text-[24px] font-semibold text-text-primary">{analytics.total}</div>
      </div>

      <div className="chart mb-[12px] rounded-md border border-border-subtle bg-bg-raised p-[14px]">
        <div className="chart-title mb-[12px] text-[10.5px] font-bold uppercase tracking-[.08em] text-text-muted">
          {analytics.byEngineTitle}
        </div>
        <div className="bar-list mt-[4px]">
          {analytics.byEngine.map((row) => (
            <div
              key={row.label}
              className="bar-item mb-[8px] flex items-center gap-[10px] font-mono text-[11px] text-text-secondary"
            >
              <span className="bar-label w-[100px] shrink-0 overflow-hidden text-ellipsis whitespace-nowrap">
                {row.label}
              </span>
              <div className="bar-track h-[6px] flex-1 overflow-hidden rounded-full bg-bg-input">
                <div
                  className="bar-fill h-full rounded-full transition-[width] duration-500 ease-ease [background-image:linear-gradient(90deg,var(--accent),var(--purple))]"
                  style={{ width: `${row.percent}%` }}
                />
              </div>
              <span className="bar-pct w-[34px] text-right text-[10.5px] text-text-muted">
                {row.percent}%
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="chart mb-[12px] rounded-md border border-border-subtle bg-bg-raised p-[14px]">
        <div className="chart-title mb-[12px] text-[10.5px] font-bold uppercase tracking-[.08em] text-text-muted">
          {analytics.limitsTitle}
        </div>
        {analytics.limits.map((limit) => (
          <div
            key={limit.name}
            className="limit-row flex items-center gap-[10px] border-t border-border-subtle py-[8px] font-mono text-[11.5px] first:border-t-0"
          >
            <span className="limit-name flex-1 text-text-secondary">{limit.name}</span>
            <span className="limit-val text-text-primary">{limit.value}</span>
            {limit.estimate === null ? null : (
              <span className="limit-est ml-[4px] text-[10.5px] text-text-muted">
                {limit.estimate}
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
