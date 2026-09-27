import { Check, Languages, LoaderCircle, Plus, ScrollText, X } from 'lucide-react';
import { useEffect, useState, type KeyboardEvent } from 'react';

import type { TaskSpec } from '../../../protocol/types';
import { strings } from '../strings';
import { cancelPendingIntent, runConfirmedIntent, runPendingAsTyped } from '../store/intents';
import { compileIntent } from '../store/kernelIntents';
import { useKernelUi } from '../store/kernelUi';
import { useAppStore } from '../store/store';
import { BTN, BTN_GHOST, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';
import { Badge } from '../panels/ui/Badge';

/**
 * **The Intent Contract card** (0.12, the Universal Intent Engine's confirm step): before an engine starts,
 * the person sees what SDC understood - in any language they wrote in - and says yes, corrects it, or
 * sends the words as typed. Nothing runs until one of those three happens (the plan's blue step).
 *
 * Five states, as every new surface has: loading (the reading is on its way), ready, a reading SDC made
 * without a model (`heuristic`, which asks rather than claims), an error (the daemon's own sentence
 * arrives as a toast and the card offers "Run as typed"), and offline (the same path).
 */
const k = strings.kernel.intent;

function Meter({ value, label }: { value: number; label: string }) {
  const percent = Math.round(value * 100);
  const tone = value >= 0.8 ? 'bg-state-success' : value >= 0.6 ? 'bg-state-waiting' : 'bg-state-error';

  return (
    <span className="inline-flex items-center gap-[6px] text-[10.5px] text-text-muted" title={k.confidenceLabel}>
      <span className="relative h-[4px] w-[46px] overflow-hidden rounded-full bg-bg-hover" aria-hidden="true">
        <span className={'absolute inset-y-0 left-0 ' + tone} style={{ width: `${percent}%` }} />
      </span>
      <span>{label}</span>
      <span className="font-mono">{k.confidence(percent)}</span>
    </span>
  );
}

export function IntentCard({ sessionId }: { sessionId: string }) {
  const pending = useKernelUi((state) => state.pending[sessionId]);
  const parsed = useAppStore((state) => (pending === undefined ? undefined : state.kernel.intents[pending.intentId]));
  const [spec, setSpec] = useState<TaskSpec | null>(null);
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [terms, setTerms] = useState<{ term: string; meaning: string }[]>([]);
  const [term, setTerm] = useState('');
  const [meaning, setMeaning] = useState('');
  const [condition, setCondition] = useState('');
  const [compiled, setCompiled] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  /* The reading arrived (or a new request replaced the last): the card starts from it. */
  useEffect(() => {
    setSpec(parsed === undefined ? null : parsed.spec);
    setAnswers({});
    setTerms([]);
    setCompiled(null);
  }, [parsed]);

  if (pending === undefined) {
    return null;
  }

  const confirm = (): void => {
    if (spec === null || busy) {
      return;
    }

    const answered = spec.questions
      .map((question, index) => (answers[index]?.trim() ? `${question} → ${answers[index]?.trim()}` : null))
      .filter((line): line is string => line !== null);
    const final: TaskSpec =
      answered.length === 0 ? spec : { ...spec, goal: { ...spec.goal, value: `${spec.goal.value}\n${answered.join('\n')}` } };

    setBusy(true);
    void runConfirmedIntent(sessionId, final, terms).finally(() => setBusy(false));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      confirm();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      cancelPendingIntent(sessionId);
    }
  };

  const footer = (
    <div className="flex flex-wrap items-center gap-[8px] border-t border-border-subtle px-[14px] py-[10px]">
      <button type="button" className={BTN + ' ' + BTN_PRIMARY} disabled={spec === null || busy} onClick={confirm} aria-keyshortcuts="Control+Enter">
        {busy ? <LoaderCircle size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={12} aria-hidden="true" />}
        {k.confirm}
      </button>
      <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={() => void runPendingAsTyped(sessionId)}>
        {k.asTyped}
      </button>
      <button type="button" className={BTN + ' ' + BTN_GHOST} onClick={() => cancelPendingIntent(sessionId)} aria-keyshortcuts="Escape">
        <X size={12} aria-hidden="true" />
        {k.cancel}
      </button>
      <span className="ml-auto text-[10.5px] text-text-muted">{k.shortcut}</span>
    </div>
  );

  if (parsed === undefined || spec === null) {
    return (
      <section className="intent-card mb-[10px] overflow-hidden rounded-lg border border-border-focus bg-bg-raised" aria-label={k.title} aria-busy="true" onKeyDown={onKeyDown}>
        <div className="flex items-center gap-[8px] px-[14px] py-[12px] text-[12.5px] text-text-secondary" role="status">
          <LoaderCircle size={14} className="animate-spin text-accent motion-reduce:animate-none" aria-hidden="true" />
          {k.reading}
        </div>
        {footer}
      </section>
    );
  }

  const language = parsed.detection;
  const setField = (field: 'target' | 'goal', value: string): void => setSpec({ ...spec, [field]: { ...spec[field], value } });

  return (
    <section className="intent-card mb-[10px] max-h-[52vh] overflow-y-auto rounded-lg border border-border-focus bg-bg-raised shadow-md" aria-label={k.title} onKeyDown={onKeyDown}>
      <header className="flex flex-wrap items-center gap-[8px] border-b border-border-subtle px-[14px] py-[10px]">
        <Languages size={14} className="text-accent" aria-hidden="true" />
        <span className="text-[12.5px] font-semibold text-text-primary">{k.title}</span>
        <Badge tone="accent">
          {k.readAs(spec.language.label || language.label)}
          {language.dialect === null ? '' : ` · ${k.dialect(language.dialect)}`}
          {language.romanized ? ` · ${k.romanized}` : ''}
          {language.mixed ? ` · ${k.mixed}` : ''}
        </Badge>
        <Badge tone={spec.source === 'heuristic' ? 'warning' : 'neutral'}>{k.source[spec.source]}</Badge>
        <Badge tone={spec.risk === 'high' ? 'warning' : 'muted'}>{k.risk[spec.risk]}</Badge>
        <span className="ml-auto">
          <Meter value={spec.confidence} label={k.overall} />
        </span>
      </header>

      <div className="flex flex-col gap-[12px] px-[14px] py-[12px] text-[12.5px]">
        {spec.summary === '' ? null : (
          <p className="leading-[1.55] text-text-primary" dir="auto">
            {spec.summary}
          </p>
        )}

        {parsed.note === null ? null : <p className="rounded-md bg-orange-subtle px-[10px] py-[6px] text-[11.5px] text-state-waiting">{parsed.note}</p>}

        {spec.showBackTranslation && spec.backTranslation !== '' ? (
          <div className="rounded-md border border-border-subtle bg-bg-base px-[10px] py-[8px]">
            <div className="mb-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.backTitle}</div>
            <p className="leading-[1.55] text-text-secondary" dir="auto">
              {spec.backTranslation}
            </p>
          </div>
        ) : null}

        {(['target', 'goal'] as const).map((field) => (
          <label key={field} className="flex flex-col gap-[4px]">
            <span className="flex items-center gap-[8px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">
              {k[field]}
              <Meter value={spec[field].confidence} label={spec.unsure.includes(field) ? k.unsure : k.sure} />
            </span>
            <textarea
              rows={field === 'goal' ? 2 : 1}
              className="w-full resize-y rounded-md border border-border-default bg-bg-input px-[9px] py-[6px] text-[12.5px] text-text-primary focus:border-border-focus"
              value={spec[field].value}
              onChange={(event) => setField(field, event.target.value)}
              dir="auto"
            />
          </label>
        ))}

        <fieldset className="flex flex-col gap-[6px]">
          <legend className="mb-[4px] flex items-center gap-[8px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">
            {k.conditions}
            <Meter value={spec.acceptanceConfidence} label={spec.unsure.includes('acceptance') ? k.unsure : k.sure} />
          </legend>
          {spec.acceptance.map((item, index) => (
            <label key={index} className="flex items-start gap-[8px]">
              <input
                type="checkbox"
                className="mt-[3px] accent-[var(--accent)]"
                checked={item.checked}
                onChange={(event) =>
                  setSpec({ ...spec, acceptance: spec.acceptance.map((entry, at) => (at === index ? { ...entry, checked: event.target.checked } : entry)) })
                }
              />
              <input
                className="min-w-0 flex-1 rounded-sm bg-transparent text-text-primary focus:bg-bg-input"
                value={item.text}
                aria-label={k.conditionLabel(index + 1)}
                onChange={(event) =>
                  setSpec({ ...spec, acceptance: spec.acceptance.map((entry, at) => (at === index ? { ...entry, text: event.target.value } : entry)) })
                }
              />
            </label>
          ))}
          <div className="flex items-center gap-[6px]">
            <input
              className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[8px] py-[4px] text-[12px]"
              placeholder={k.addCondition}
              value={condition}
              onChange={(event) => setCondition(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !event.ctrlKey && condition.trim() !== '') {
                  event.preventDefault();
                  setSpec({ ...spec, acceptance: [...spec.acceptance, { text: condition.trim(), checked: true }] });
                  setCondition('');
                }
              }}
            />
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_SECONDARY}
              disabled={condition.trim() === ''}
              aria-label={k.addCondition}
              onClick={() => {
                setSpec({ ...spec, acceptance: [...spec.acceptance, { text: condition.trim(), checked: true }] });
                setCondition('');
              }}
            >
              <Plus size={11} aria-hidden="true" />
            </button>
          </div>
        </fieldset>

        {spec.outOfScope.length === 0 ? null : (
          <div>
            <div className="mb-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.outOfScope}</div>
            <ul className="flex flex-wrap gap-[6px]">
              {spec.outOfScope.map((item) => (
                <li key={item} className="rounded-full bg-bg-hover px-[8px] py-[2px] text-[11px] text-text-secondary">
                  {item}
                </li>
              ))}
            </ul>
          </div>
        )}

        {spec.questions.length === 0 ? null : (
          <div className="flex flex-col gap-[6px] rounded-md border border-state-waiting px-[10px] py-[8px]">
            <div className="text-[10px] font-semibold uppercase tracking-[.1em] text-state-waiting">{k.questions}</div>
            {spec.questions.map((question, index) => (
              <label key={question} className="flex flex-col gap-[4px]">
                <span className="text-text-primary" dir="auto">
                  {question}
                </span>
                <input
                  className="rounded-md border border-border-default bg-bg-input px-[8px] py-[4px] text-[12px]"
                  placeholder={k.answer}
                  value={answers[index] ?? ''}
                  onChange={(event) => setAnswers({ ...answers, [index]: event.target.value })}
                  dir="auto"
                />
              </label>
            ))}
          </div>
        )}

        <details className="rounded-md border border-border-subtle px-[10px] py-[6px]">
          <summary className="cursor-pointer text-[11.5px] text-text-secondary">{k.glossaryTitle}</summary>
          <div className="mt-[6px] flex flex-wrap items-center gap-[6px]">
            <input className="w-[120px] rounded-md border border-border-default bg-bg-input px-[8px] py-[3px] text-[12px]" placeholder={k.term} value={term} onChange={(event) => setTerm(event.target.value)} />
            <span className="text-text-muted">=</span>
            <input className="w-[160px] rounded-md border border-border-default bg-bg-input px-[8px] py-[3px] text-[12px]" placeholder={k.meaning} value={meaning} onChange={(event) => setMeaning(event.target.value)} />
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_SECONDARY}
              disabled={term.trim() === '' || meaning.trim() === ''}
              onClick={() => {
                setTerms([...terms, { term: term.trim(), meaning: meaning.trim() }]);
                setTerm('');
                setMeaning('');
              }}
            >
              {k.addTerm}
            </button>
          </div>
          {terms.length === 0 ? null : (
            <ul className="mt-[6px] flex flex-wrap gap-[6px]">
              {terms.map((entry) => (
                <li key={entry.term} className="rounded-full bg-accent-subtle px-[8px] py-[2px] text-[11px] text-accent">
                  {entry.term} = {entry.meaning}
                </li>
              ))}
            </ul>
          )}
        </details>

        <div>
          <button
            type="button"
            className={BTN_SM + ' ' + BTN_GHOST}
            onClick={() => {
              if (compiled !== null) {
                setCompiled(null);

                return;
              }

              void compileIntent(pending.intentId, pending.seed.engine, sessionId).then(setCompiled);
            }}
          >
            <ScrollText size={11} aria-hidden="true" />
            {compiled === null ? k.showPrompt : k.hidePrompt}
          </button>
          {compiled === null ? null : (
            <>
              <p className="mt-[4px] text-[10.5px] text-text-muted">{k.compiledNote(pending.seed.engine)}</p>
              <pre className="mt-[4px] max-h-[220px] overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-bg-input px-[10px] py-[8px] font-mono text-[11px] leading-[1.5] text-text-secondary" dir="ltr">
                {compiled}
              </pre>
            </>
          )}
        </div>
      </div>

      {footer}
    </section>
  );
}
