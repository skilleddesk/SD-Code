import { FileDown, History, Link2, ShieldAlert, ShieldCheck, ShieldQuestion } from 'lucide-react';
import { useMemo, useState } from 'react';

import type { AuditEntry, LedgerCheck } from '../../../protocol/types';
import { currentLocale } from '../i18n';
import { openOutside } from '../lib/external';
import { strings } from '../strings';
import { rewindTo, runVerify, defaultReviewer } from '../store/intents';
import { measured, usd } from '../store/kernel';
import { exportProof, loadAudit } from '../store/kernelIntents';
import { useSessionsStore } from '../store/sessions';
import { useAppStore } from '../store/store';
import { BTN, BTN_BLOCK, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * **The Proof Panel** (0.12, the pipeline's last step): for one turn, everything that says whether to
 * trust it - what it changed and ran, how it was checked (tests, secret and SAST scans, dependencies, a
 * second AI against the agreed conditions), its Trust score with every reason, what it cost and whether
 * that number was measured, the point to roll back to, and the audit ledger's rows for it. "Export" writes
 * the same as a Proof Pack a client can open with no software.
 */
const k = strings.kernel.proof;

function Title({ children }: { children: React.ReactNode }) {
  return <div className="mb-[6px] mt-[14px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted first:mt-0">{children}</div>;
}

export function ProofTab() {
  const { activeTab: sessionId } = useSessionsStore();
  const allTurns = useAppStore((state) => state.turns);
  const turns = useMemo(() => allTurns.filter((turn) => turn.sessionId === sessionId), [allTurns, sessionId]);
  const [chosen, setChosen] = useState<string | null>(null);
  const turn = turns.find((candidate) => candidate.id === chosen) ?? turns.at(-1) ?? null;
  const score = useAppStore((state) => (turn === null ? undefined : state.kernel.scores[turn.id]));
  const cost = useAppStore((state) => (turn === null ? undefined : state.kernel.costs[turn.id]));
  const verifies = useAppStore((state) => state.verifies);
  const checkpoints = useAppStore((state) => state.checkpoints);
  const [audit, setAudit] = useState<{ entries: AuditEntry[]; chain: LedgerCheck } | null>(null);
  const [exported, setExported] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  if (sessionId === null || turn === null) {
    return <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] text-text-muted">{k.empty}</div>;
  }

  const verify = [...verifies].reverse().find((run) => run.turnId === turn.id && run.state === 'done') ?? null;
  const rollback = checkpoints.find((checkpoint) => checkpoint.turnId === turn.id) ?? null;
  const files = turn.tools.filter((tool) => tool.tool === 'edit');
  const commands = turn.tools.filter((tool) => tool.tool === 'run');
  const Icon = score?.level === 'high' ? ShieldCheck : score?.level === 'medium' ? ShieldQuestion : ShieldAlert;
  const tone = score?.level === 'high' ? 'border-state-success text-state-success' : score?.level === 'medium' ? 'border-state-waiting text-state-waiting' : 'border-state-error text-state-error';

  return (
    <div className="proof flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto p-[12px] text-[12px]">
        <label className="mb-[10px] flex flex-col gap-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted" htmlFor="proof-turn">
          {k.turn}
          <span className="flex h-[30px] rounded-md border border-border-default bg-bg-input focus-within:border-border-focus">
            <select
              id="proof-turn"
              className="h-full w-full bg-transparent px-[8px] text-[12px] font-normal normal-case tracking-normal text-text-primary outline-none [&>option]:bg-bg-overlay"
              value={turn.id}
              onChange={(event) => {
                setChosen(event.target.value);
                setAudit(null);
                setExported(null);
              }}
            >
              {turns.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {`#${candidate.turnNumber} · ${candidate.prompt.slice(0, 60)}`}
                </option>
              ))}
            </select>
          </span>
        </label>

        <Title>{k.trust}</Title>
        {score === undefined ? (
          <p className="text-text-muted">{k.noScore}</p>
        ) : (
          <div className={'rounded-md border px-[12px] py-[9px] ' + tone}>
            <div className="flex items-center gap-[8px] text-[13px] font-semibold">
              <Icon size={15} aria-hidden="true" />
              {strings.kernel.trust.chip(score.score, strings.kernel.trust.level[score.level])}
            </div>
            <ul className="mt-[6px] flex flex-col gap-[3px] text-text-secondary">
              {score.reasons.map((reason) => (
                <li key={reason.text} className="flex gap-[6px]">
                  <span className="w-[34px] shrink-0 text-right font-mono text-[10.5px]">{reason.delta === 0 ? '·' : reason.delta}</span>
                  <span>{reason.text}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <Title>{k.changed(files.length)}</Title>
        {files.length === 0 ? <p className="text-text-muted">{k.nothing}</p> : null}
        <ul className="flex flex-col gap-[2px] font-mono text-[11px]" dir="ltr">
          {files.map((file) => (
            <li key={file.callId} className="truncate text-text-primary" title={file.target}>
              {file.name} {file.target} <span className="text-text-muted">{file.meta}</span>
            </li>
          ))}
        </ul>

        <Title>{k.commands(commands.length)}</Title>
        {commands.length === 0 ? <p className="text-text-muted">{k.nothing}</p> : null}
        <ul className="flex flex-col gap-[2px] font-mono text-[11px]" dir="ltr">
          {commands.map((command) => (
            <li key={command.callId} className="truncate" title={command.target}>
              <span className={command.status === 'failed' ? 'text-state-error' : 'text-text-primary'}>{command.target}</span>{' '}
              <span className="text-text-muted">{command.meta}</span>
            </li>
          ))}
        </ul>

        <Title>{k.verification}</Title>
        {verify === null ? (
          <div className="flex flex-col gap-[6px]">
            <p className="text-state-waiting">{k.unverified}</p>
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_SECONDARY + ' self-start'}
              onClick={() => void runVerify({ sessionId, turnId: turn.id, reviewer: defaultReviewer(turn) })}
            >
              {k.verifyNow}
            </button>
          </div>
        ) : (
          <div className="flex flex-col gap-[6px]">
            <div className="font-semibold">{k.verdict[verify.verdict ?? (verify.pass === true ? 'PASS' : 'FAIL')]}</div>
            <ul className="font-mono text-[11px]" dir="ltr">
              {verify.checks.map((check) => (
                <li key={check.command} className={check.status === 'fail' ? 'text-state-error' : 'text-text-secondary'}>
                  {check.status} · {check.command}
                </li>
              ))}
            </ul>
            {verify.scans === undefined ? null : (
              <p className="text-text-secondary">
                {k.scans(verify.scans.secrets?.length ?? 0, verify.scans.sast?.length ?? 0)}
                {verify.scans.dependencies?.counts === undefined
                  ? ` · ${k.depsUnchecked}`
                  : ` · ${k.deps(verify.scans.dependencies.counts.critical + verify.scans.dependencies.counts.high)}`}
              </p>
            )}
            {[...(verify.scans?.secrets ?? []), ...(verify.scans?.sast ?? [])].slice(0, 8).map((finding) => (
              <p key={`${finding.rule}${finding.file}${finding.line}`} className="text-[11px] text-state-error">
                <span className="font-mono" dir="ltr">
                  {finding.file}:{finding.line}
                </span>{' '}
                {finding.message}
              </p>
            ))}
            {verify.review === null || verify.review.verdict === undefined || verify.review.verdict === null ? null : (
              <p className="text-text-secondary">{k.review(verify.review.model || verify.review.engine, verify.review.verdict)}</p>
            )}
            {(verify.review?.criteria ?? []).map((criterion, index) => (
              <p key={index} className={criterion.met === false ? 'text-state-error' : 'text-state-success'}>
                {criterion.met === false ? '✗' : '✓'} {criterion.why}
              </p>
            ))}
          </div>
        )}

        <Title>{k.cost}</Title>
        {cost === undefined ? (
          <p className="text-text-muted">{turn.estimate?.usd === undefined || turn.estimate.usd === null ? k.noCost : k.estimateOnly(usd(turn.estimate.usd))}</p>
        ) : (
          <p className="text-text-secondary">
            {usd(cost.costUsd)} · {strings.kernel.cost.source[cost.costSource]} · {k.tokens(cost.inputTokens, cost.outputTokens)}
            {turn.estimate?.usd === undefined || turn.estimate.usd === null ? '' : ` · ${k.estimateWas(usd(turn.estimate.usd))}`}
            {measured(cost.costSource) ? '' : ` · ${k.notMeasured}`}
          </p>
        )}

        <Title>{k.rollback}</Title>
        {rollback === null ? (
          <p className="text-text-muted">{files.length === 0 && commands.length === 0 ? k.noChangeNoCheckpoint : k.noCheckpoint}</p>
        ) : (
          <div className="flex flex-col gap-[6px]">
            <p className="text-text-secondary">
              {rollback.label ?? rollback.title}
              {rollback.irreversible === null || rollback.irreversible === undefined ? null : (
                <span className="block text-[11px] text-state-waiting">{strings.kernel.timeline.irreversible(rollback.irreversible)}</span>
              )}
            </p>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' self-start'} onClick={() => void rewindTo(sessionId, `turn-${rollback.turn}`)}>
              <History size={11} aria-hidden="true" />
              {k.rewind}
            </button>
          </div>
        )}

        <Title>{k.ledger}</Title>
        {audit === null ? (
          <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void loadAudit({ turnId: turn.id, limit: 200 }).then(setAudit)}>
            <Link2 size={11} aria-hidden="true" />
            {k.showLedger}
          </button>
        ) : (
          <div className="flex flex-col gap-[4px]">
            <p className={audit.chain.intact ? 'text-state-success' : 'text-state-error'}>
              {audit.chain.intact ? k.intact(audit.chain.entries) : k.broken(audit.chain.brokenAt ?? 0)}
            </p>
            <ol className="flex flex-col gap-[2px] text-[11px]">
              {[...audit.entries].reverse().map((entry) => (
                <li key={entry.seq} className="flex gap-[6px] text-text-secondary">
                  <span className="shrink-0 font-mono text-text-muted">#{entry.seq}</span>
                  <span className="min-w-0 truncate" title={`${entry.actor} · ${entry.hash}`}>
                    {entry.summary}
                  </span>
                </li>
              ))}
            </ol>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-[6px] border-t border-border-subtle p-[12px]">
        {exported === null ? null : (
          <button type="button" className="truncate text-left font-mono text-[10.5px] text-accent hover:underline" dir="ltr" onClick={() => void openOutside(`file:///${exported.replace(/\\/g, '/')}`)}>
            {exported}
          </button>
        )}
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY + ' ' + BTN_BLOCK}
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void exportProof(sessionId, turn.id, currentLocale())
              .then((result) => {
                if (result !== null) {
                  setExported(result.htmlPath);
                  void openOutside(`file:///${result.htmlPath.replace(/\\/g, '/')}`);
                }
              })
              .finally(() => setBusy(false));
          }}
        >
          <FileDown size={12} aria-hidden="true" />
          {k.export}
        </button>
      </div>
    </div>
  );
}
