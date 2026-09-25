import {
  CircleCheckBig,
  CircleDashed,
  CircleX,
  FileCode,
  LoaderCircle,
  Play,
  ScanSearch,
  Wand2,
} from 'lucide-react';
import { useMemo, useState } from 'react';

import type { VerifyCheck, VerifyIssue } from '../../../../protocol/types';
import { strings } from '../../strings';
import { defaultReviewer, openFile, reviewerOptions, runVerify, type ReviewerOption } from '../../store/intents';
import { useFilesStore } from '../../store/files';
import { useModelStore } from '../../store/model';
import { useSessionsStore } from '../../store/sessions';
import { useAppStore } from '../../store/store';
import type { VerifyView } from '../../store/types';
import { BTN, BTN_BLOCK, BTN_PRIMARY, BTN_SECONDARY } from '../ui/button';

/**
 * The Verify tab (spec section 7.11, rebuilt in v4).
 *
 * Two stages, drawn in the order they run:
 *
 *   CHECKS   what the project itself says "working" means - its own typecheck, lint, test and build
 *            scripts (or cargo / go / pytest), run in the chat's folder. Each row: a status icon, the
 *            command in mono, how long it took, and the tail of the output when it failed.
 *   REVIEW   a different AI reads the change (since the turn's checkpoint, new files included) and
 *            answers with a verdict and issues pinned to file:line. Each issue opens its file, and
 *            `Fix with a prompt` puts the fix request in the prompt box for the person to send.
 *
 * Until v4 this tab drew four fixed rows (`test · rate.test.ts` failing, every time) and its button
 * toasted "Verify — 3 pass, 1 fail". Everything here is the daemon's `VerifyUpdated` snapshot.
 */
export function VerifyTab() {
  const { activeTab: sessionId } = useSessionsStore();
  const verifies = useAppStore((state) => state.verifies);
  const turns = useAppStore((state) => state.turns);
  const providers = useAppStore((state) => state.providers);
  const catalog = useModelStore((state) => state.catalog);
  const run = useMemo(
    () => [...verifies].reverse().find((candidate) => candidate.sessionId === sessionId) ?? null,
    [verifies, sessionId],
  );
  const author = useMemo(
    () => [...turns].reverse().find((turn) => turn.sessionId === sessionId) ?? null,
    [turns, sessionId],
  );
  /* The reviewers on offer are the connected providers - re-read when either list changes. */
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const options = useMemo(() => reviewerOptions(), [providers, catalog]);
  const [chosen, setChosen] = useState<string | null>(null);
  const reviewer: ReviewerOption | null =
    options.find((option) => key(option) === chosen) ?? defaultReviewer(author) ?? null;
  const running = run?.state === 'running';

  const start = (reviewFailing = false): void => {
    if (sessionId !== null) {
      void runVerify({ sessionId, reviewer, reviewFailing });
    }
  };

  return (
    <div className="verify flex min-h-0 flex-1 flex-col">
      <div className="verify-body min-h-0 flex-1 overflow-y-auto p-[12px]">
        {run === null ? (
          <div className="px-[12px] py-[28px] text-center text-[12.5px] leading-[1.6] text-text-muted">
            {sessionId === null ? strings.rightPanel.verify.noFolder : strings.rightPanel.verify.empty}
          </div>
        ) : (
          <RunView run={run} onReviewAnyway={() => start(true)} />
        )}
      </div>

      <div className="verify-footer flex flex-col gap-[8px] border-t border-border-subtle p-[12px]">
        <label className="flex flex-col gap-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted" htmlFor="verify-reviewer">
          {strings.rightPanel.verify.reviewer}
          <select
            id="verify-reviewer"
            className="h-[30px] rounded-md border border-border-default bg-bg-input px-[8px] text-[12px] font-normal normal-case tracking-normal text-text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-border-focus"
            value={reviewer === null ? '' : key(reviewer)}
            disabled={options.length === 0}
            onChange={(event) => setChosen(event.target.value)}
          >
            {options.length === 0 ? <option value="">{strings.rightPanel.verify.noReviewer}</option> : null}
            {options.map((option) => (
              <option key={key(option)} value={key(option)}>
                {option.label}
                {author !== null && option.engine === author.engine && option.model === author.model ? ' (wrote it)' : ''}
              </option>
            ))}
          </select>
        </label>

        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY + ' ' + BTN_BLOCK}
          disabled={sessionId === null || running}
          onClick={() => start(false)}
        >
          {running ? <LoaderCircle size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Play size={12} aria-hidden="true" />}
          {running ? strings.rightPanel.verify.running : run === null ? strings.rightPanel.verify.run : strings.rightPanel.verify.again}
        </button>
      </div>
    </div>
  );
}

function key(option: ReviewerOption): string {
  return `${option.provider}:${option.model}`;
}

function RunView({ run, onReviewAnyway }: { run: VerifyView; onReviewAnyway: () => void }) {
  return (
    <div className="flex flex-col gap-[14px]">
      {run.state === 'done' && run.pass !== null ? (
        <div
          className={
            'verify-verdict flex items-center gap-[8px] rounded-md border px-[12px] py-[9px] text-[12.5px] font-semibold ' +
            (run.pass ? 'border-state-success bg-green-subtle text-state-success' : 'border-state-error bg-red-subtle text-state-error')
          }
          role="status"
        >
          {run.pass ? <CircleCheckBig size={14} aria-hidden="true" /> : <CircleX size={14} aria-hidden="true" />}
          {run.pass ? strings.rightPanel.verify.passed : strings.rightPanel.verify.notPassed}
        </div>
      ) : null}

      <section aria-label={strings.rightPanel.verify.checksTitle}>
        <SectionTitle>{strings.rightPanel.verify.checksTitle}</SectionTitle>
        {run.checks.length === 0 ? (
          <p className="text-[12px] leading-[1.55] text-text-muted">{run.note === '' ? strings.rightPanel.verify.noChecks : run.note}</p>
        ) : (
          <div className="flex flex-col gap-[6px]">
            {run.checks.map((check) => (
              <CheckRow key={check.command} check={check} />
            ))}
          </div>
        )}
      </section>

      {run.review === null ? null : (
        <section aria-label={strings.rightPanel.verify.reviewTitle}>
          <SectionTitle>{strings.rightPanel.verify.reviewTitle}</SectionTitle>
          <Review review={run.review} onReviewAnyway={onReviewAnyway} />
        </section>
      )}
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <div className="mb-[6px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{children}</div>;
}

function StatusIcon({ status }: { status: VerifyCheck['status'] }) {
  switch (status) {
    case 'pass':
      return <CircleCheckBig size={14} className="shrink-0 text-state-success" aria-hidden="true" />;
    case 'fail':
      return <CircleX size={14} className="shrink-0 text-state-error" aria-hidden="true" />;
    case 'running':
      return <LoaderCircle size={14} className="shrink-0 animate-spin text-accent motion-reduce:animate-none" aria-hidden="true" />;
    default:
      return <CircleDashed size={14} className="shrink-0 text-text-faint" aria-hidden="true" />;
  }
}

function CheckRow({ check }: { check: VerifyCheck }) {
  const [open, setOpen] = useState(check.status === 'fail');
  const hasTail = check.tail.length > 0 && check.status === 'fail';

  return (
    <div className="verify-row overflow-hidden rounded-md border border-border-subtle bg-bg-raised" data-status={check.status}>
      <button
        type="button"
        className="flex w-full items-center gap-[10px] px-[12px] py-[9px] text-left disabled:cursor-default"
        disabled={!hasTail}
        aria-expanded={hasTail ? open : undefined}
        onClick={() => setOpen((current) => !current)}
      >
        <StatusIcon status={check.status} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-mono text-[11.5px] text-text-primary">{check.command}</span>
          <span className="sr-only">{strings.rightPanel.verify.status[check.status]}</span>
        </span>
        <span className="shrink-0 font-mono text-[10.5px] tabular-nums text-text-muted">
          {check.ms === null ? strings.rightPanel.verify.status[check.status] : strings.rightPanel.verify.ms(check.ms)}
        </span>
      </button>
      {hasTail && open ? (
        <pre className="max-h-[180px] overflow-auto border-t border-border-subtle bg-bg-input px-[12px] py-[8px] font-mono text-[10.5px] leading-[1.55] text-text-secondary">
          {check.tail.join('\n')}
        </pre>
      ) : null}
    </div>
  );
}

function Review({ review, onReviewAnyway }: { review: NonNullable<VerifyView['review']>; onReviewAnyway: () => void }) {
  const issues = review.issues ?? [];
  const verdict =
    review.verdict === 'issues'
      ? strings.rightPanel.verify.verdict.issues(issues.length)
      : review.verdict === 'unreadable'
        ? strings.rightPanel.verify.verdict.unreadable
        : review.verdict === 'pass'
          ? strings.rightPanel.verify.verdict.pass
          : strings.rightPanel.verify.reviewStatus[review.status];

  return (
    <div className="verdict overflow-hidden rounded-md border border-border-default bg-bg-raised">
      <div className="flex items-center gap-[9px] border-b border-border-subtle px-[12px] py-[9px] text-[12px]">
        {review.status === 'running' ? (
          <LoaderCircle size={13} className="animate-spin text-accent motion-reduce:animate-none" aria-hidden="true" />
        ) : (
          <ScanSearch size={13} className="text-text-muted" aria-hidden="true" />
        )}
        <span className="min-w-0 truncate font-semibold text-text-primary">{review.model === '' ? review.engine : review.model}</span>
        <span className="font-mono text-[10.5px] text-text-muted">{review.engine}</span>
        {verdict === '' ? null : (
          <span
            className={
              'ml-auto shrink-0 rounded-full px-[9px] py-[2px] text-[10.5px] font-semibold ' +
              (review.verdict === 'pass'
                ? 'bg-green-subtle text-state-success'
                : review.verdict === 'issues'
                  ? 'bg-orange-subtle text-state-waiting'
                  : 'bg-bg-hover text-text-muted')
            }
          >
            {verdict}
          </span>
        )}
      </div>

      {review.summary === undefined || review.summary === '' ? null : (
        <p className="px-[12px] pt-[9px] text-[12px] leading-[1.55] text-text-secondary">{review.summary}</p>
      )}

      {review.status === 'skipped' ? (
        <div className="px-[12px] pb-[10px] pt-[8px]">
          <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={onReviewAnyway}>
            {strings.rightPanel.verify.reviewAnyway}
          </button>
        </div>
      ) : null}

      {issues.length === 0 ? <div className="h-[9px]" /> : (
        <ul className="flex flex-col divide-y divide-border-subtle">
          {issues.map((issue, index) => (
            <IssueRow key={`${issue.file}:${issue.line ?? ''}:${index}`} issue={issue} />
          ))}
        </ul>
      )}
    </div>
  );
}

const SEVERITY = {
  high: 'bg-red-subtle text-state-error',
  medium: 'bg-orange-subtle text-state-waiting',
  low: 'bg-bg-hover text-text-muted',
} as const;

function IssueRow({ issue }: { issue: VerifyIssue }) {
  const root = useFilesStore((state) => state.root);
  const where = `${issue.file}${issue.line === null ? '' : `:${issue.line}`}`;

  const open = (): void => {
    if (root === null || issue.file === '') {
      return;
    }

    const separator = root.includes('\\') ? '\\' : '/';
    const path = /^([a-zA-Z]:[\\/]|\/)/.test(issue.file) ? issue.file : `${root}${separator}${issue.file.replace(/[\\/]/g, separator)}`;

    void openFile(path, issue.file.split(/[\\/]/).pop() ?? issue.file);
  };

  return (
    <li className="issue flex flex-col gap-[7px] px-[12px] py-[10px]">
      <div className="flex items-center gap-[8px]">
        <span className={'rounded-sm px-[6px] py-[1px] text-[9.5px] font-bold uppercase tracking-[.06em] ' + SEVERITY[issue.severity]}>
          {strings.rightPanel.verify.severity[issue.severity]}
        </span>
        <button
          type="button"
          className="flex min-w-0 items-center gap-[5px] font-mono text-[11px] text-accent hover:underline disabled:text-text-muted disabled:no-underline"
          disabled={root === null || issue.file === ''}
          onClick={open}
        >
          <FileCode size={11} aria-hidden="true" className="shrink-0" />
          <span className="truncate">{where === '' ? '—' : where}</span>
        </button>
      </div>
      <p className="text-[12px] leading-[1.55] text-text-secondary">{issue.message}</p>
      {issue.fix === '' ? null : <p className="text-[11.5px] leading-[1.5] text-text-muted">{issue.fix}</p>}
      <button
        type="button"
        className="flex items-center gap-[6px] self-start rounded-sm border border-border-focus bg-accent-subtle px-[10px] py-[4px] text-[11px] font-semibold text-accent hover:bg-accent-fill hover:text-text-on-accent"
        onClick={() => useModelStore.getState().setDraft(strings.rightPanel.verify.fixPrompt(issue))}
      >
        <Wand2 size={11} aria-hidden="true" />
        {strings.rightPanel.verify.fix}
      </button>
    </li>
  );
}
