import { ProofBadge } from '../../kernel/ProofBadge';
import { TrustChip } from '../../kernel/TrustChip';
import { CircleCheckBig, CircleX, GitBranch, LoaderCircle, ScanSearch, ThumbsDown, ThumbsUp } from 'lucide-react';
import { useEffect, useState } from 'react';

import { strings } from '../../strings';
import { openOutside } from '../../lib/external';
import { pushToGitHub, useGitHubStore } from '../../store/github';
import { runVerifyChosen } from '../../store/intents';
import { useOverlayStore } from '../../store/overlays';
import { toast } from '../../store/toast';
import { receiptOf } from './relay';
import type { RelaySummaryEvent } from '../../../../protocol/types';
import type { TurnFooterData, TurnVerifyData } from './types';

/**
 * `.turn-footer` - what the turn cost, and what you thought of it (spec section 7.5, 19.4).
 *
 * A hairline above, then a status pill (`✓ Done`) and the turn's numbers as chips (`3 steps`, `12k in`) on the
 * left - never a price (1.0.0) - and the feedback buttons on the right. The verb is weighted (`Done` in --text-primary, the numbers muted) because
 * the numbers are reference material, not the message.
 *
 * Feedback is one-of-two, never both and never neither-forever: clicking the active button again
 * clears it, which is the only way back to "unrated". The choice is local state - there is no
 * backend to send it to yet, and the toast is the whole acknowledgement the spec asks for
 * (section 9.8 leaves the notification itself to a later step).
 */
export interface TurnFooterProps {
  footer: TurnFooterData;
  /** The Verify chip's facts; `null` while the turn is still running (there is nothing to verify yet). */
  verify?: {
    sessionId: string;
    turnId: string;
    author: { engine: string; model: string };
    result: TurnVerifyData | null;
  } | null;
  /** Lines the turn added plus removed (P0-15's badge), or `null` when it changed nothing. */
  lines?: number | null;
  /** Silent Relay's split of the turn (M1-7): the receipt lines. `null` when the turn was not in that mode. */
  relay?: RelaySummaryEvent | null;
}

type Feedback = 'up' | 'down' | null;

export function TurnFooter({ footer, verify = null, lines = null, relay = null }: TurnFooterProps) {
  const [feedback, setFeedback] = useState<Feedback>(null);

  const rate = (direction: Exclude<Feedback, null>): void => {
    const next = feedback === direction ? null : direction;

    setFeedback(next);

    if (next !== null) {
      toast(
        next === 'up' ? strings.turns.footer.upToast : strings.turns.footer.downToast,
      );
    }
  };

  /*
   * The totals line, and the case that produced a bare `·` on screen.
   *
   * `TurnStarted` used to carry a fixed price, so `summary` and `detail` were never both empty and the
   * separator always had something to join. They are both empty until `TurnCompleted` arrives now - and
   * a separator with nothing on either side of it is a defect you can see. So the line is drawn only
   * when there is a line to draw; the parts that exist are joined, not concatenated.
   */
  const totals = [footer.summary, footer.detail].filter((part) => part.trim() !== '');

  return (
    <>
    <div className="turn-footer mt-[8px] flex flex-wrap items-center gap-[10px] border-t border-border-subtle py-[9px] text-[11.5px] tabular-nums text-text-muted">
      <div className="summary flex min-w-[180px] flex-1 flex-wrap items-center gap-[6px]">
        {totals.length === 0 ? (
          <span className="text-text-muted">{strings.turns.footer.pending}</span>
        ) : (
          <>
            {footer.summary.trim() === '' ? null : <StatusPill summary={footer.summary} />}
            {chipsOf(footer.detail).map((chip) => (
              <span
                key={chip}
                className="footer-chip inline-flex h-[20px] items-center rounded-full border border-border-subtle bg-bg-raised px-[8px] font-mono text-[10.5px] text-text-muted"
              >
                {chip}
              </span>
            ))}
          </>
        )}
      </div>

      {verify === null ? null : <TrustChip turnId={verify.turnId} sessionId={verify.sessionId} />}

      {verify === null ? null : <VerifyChip {...verify} />}

      {verify === null || lines === null || lines === 0 ? null : <PushChip sessionId={verify.sessionId} />}

      <div className="feedback flex gap-[2px]">
        <button
          type="button"
          className={
            'feedback-btn grid h-[26px] w-[26px] place-items-center rounded-sm transition-all duration-fast ease-ease ' +
            (feedback === 'up'
              ? 'active up bg-green-subtle text-state-success'
              : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
          }
          title={strings.turns.footer.up}
          aria-label={strings.turns.footer.up}
          aria-pressed={feedback === 'up'}
          onClick={() => rate('up')}
        >
          <ThumbsUp size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={
            'feedback-btn grid h-[26px] w-[26px] place-items-center rounded-sm transition-all duration-fast ease-ease ' +
            (feedback === 'down'
              ? 'active down bg-red-subtle text-state-error'
              : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
          }
          title={strings.turns.footer.down}
          aria-label={strings.turns.footer.down}
          aria-pressed={feedback === 'down'}
          onClick={() => rate('down')}
        >
          <ThumbsDown size={13} aria-hidden="true" />
        </button>
      </div>
    </div>
    {relay === null || receiptOf(relay).length === 0 ? null : (
      <div className="relay-receipt mb-[6px] flex flex-col gap-[1px] text-[11px] leading-[1.5] text-text-muted" data-relay-receipt="">
        {receiptOf(relay).map((line) => (
          <span key={line}>{line}</span>
        ))}
      </div>
    )}
    {verify === null ? null : <ProofBadge turnId={verify.turnId} verify={verify.result} lines={lines} />}
    </>
  );
}

/** The detail line cut into chips: `3 steps · 12k in · 2k out` -> three chips. A price never reaches here (1.0.0). */
export function chipsOf(detail: string): string[] {
  return detail
    .split(' · ')
    .map((part) => part.trim())
    .filter((part) => part !== '' && !part.includes('$'));
}

/** How a turn ended, as a coloured pill: done is green, a stop that needs the person is amber. */
function StatusPill({ summary }: { summary: string }) {
  const done = /^done(?![a-z])/i.test(summary.trim());
  const tone = done ? 'border-state-success bg-green-subtle text-state-success' : 'border-state-waiting bg-orange-subtle text-state-waiting';

  return (
    <strong className={'status-pill inline-flex h-[22px] items-center gap-[5px] rounded-full border px-[10px] text-[11.5px] font-semibold ' + tone}>
      {done ? <CircleCheckBig size={12} aria-hidden="true" /> : <CircleX size={12} aria-hidden="true" />}
      {summary}
    </strong>
  );
}

/**
 * One click from a finished change to GitHub (1.0.0): pushes this chat's folder - on this computer or on its VPS.
 * Without a connected account it opens Settings → GitHub, where Connect is one approval on GitHub.
 */
function PushChip({ sessionId }: { sessionId: string }) {
  const connected = useGitHubStore((state) => state.status?.connected === true);
  const refresh = useGitHubStore((state) => state.refresh);
  const [busy, setBusy] = useState(false);
  const [url, setUrl] = useState<string | null>(null);
  const words = strings.kernel.github;

  useEffect(() => {
    if (useGitHubStore.getState().status === null) {
      void refresh();
    }
  }, [refresh]);

  const base =
    'push-chip inline-flex h-[24px] items-center gap-[6px] rounded-full border px-[10px] text-[11px] font-medium transition-colors duration-fast';

  if (url !== null) {
    return (
      <button type="button" className={base + ' border-state-success bg-green-subtle text-state-success'} onClick={() => void openOutside(url)}>
        <CircleCheckBig size={11} aria-hidden="true" />
        {words.open}
      </button>
    );
  }

  return (
    <button
      type="button"
      data-push-chip=""
      disabled={busy}
      className={base + ' border-border-default text-text-secondary hover:border-border-strong hover:text-text-primary'}
      onClick={() => {
        if (!connected) {
          useOverlayStore.getState().openSettings('github');
          return;
        }

        setBusy(true);
        void pushToGitHub({ sessionId })
          .then((answer) => {
            if ('url' in answer) {
              setUrl(answer.url);
              toast(words.pushed(answer.url));
            } else {
              toast(words.pushFailed(answer.error), undefined, 8000);
            }
          })
          .finally(() => setBusy(false));
      }}
    >
      {busy ? <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <GitBranch size={11} aria-hidden="true" />}
      {busy ? words.pushing : connected ? words.footerPush : words.footerConnect}
    </button>
  );
}

/**
 * `Verify with…` (v4, docs/ROADMAP-v4.md §3 #6): one click runs this turn's checks and has another AI
 * review its change; once a run exists the chip shows its outcome, and a click runs it again.
 */
function VerifyChip({ sessionId, turnId, result }: NonNullable<TurnFooterProps['verify']>) {
  const run = (): void => {
    void runVerifyChosen({ sessionId, turnId });
  };
  const base =
    'verify-chip inline-flex h-[24px] items-center gap-[6px] rounded-full border px-[10px] text-[11px] font-medium transition-colors duration-fast';

  if (result === null) {
    return (
      <button type="button" className={base + ' border-border-default text-text-secondary hover:border-border-strong hover:text-text-primary'} onClick={run}>
        <ScanSearch size={11} aria-hidden="true" />
        {strings.rightPanel.verify.footerChip}
      </button>
    );
  }

  if (result.state === 'running') {
    return (
      <span className={base + ' border-border-default text-text-muted'} role="status">
        <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
        {strings.rightPanel.verify.running}
      </span>
    );
  }

  const pass = result.pass === true;

  return (
    <button
      type="button"
      className={base + ' ' + (pass ? 'border-state-success bg-green-subtle text-state-success' : 'border-state-waiting bg-orange-subtle text-state-waiting')}
      title={strings.rightPanel.verify.again}
      onClick={run}
    >
      {pass ? <CircleCheckBig size={11} aria-hidden="true" /> : <CircleX size={11} aria-hidden="true" />}
      {strings.rightPanel.verify.footerResult(pass, result.reviewer, result.issues)}
    </button>
  );
}
