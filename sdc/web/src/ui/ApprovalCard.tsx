import { useState } from 'react';
import { needsFreshPasskey, secondsLeft, type Card } from '../crypto/approval';
import type { Answer } from '../transport/link';
import { useModel, useNow, useT } from './hooks';

/**
 * One request waiting for the person. What the card shows is exactly what the daemon will run, and what is signed is the
 * hash of exactly that. Dangerous work asks for the passkey again each time; "allow for a while" is not offered for it.
 */
export function ApprovalCard({ card }: { card: Card }) {
  const t = useT();
  const model = useModel();
  const now = useNow();
  const [busy, setBusy] = useState(false);
  const [reason, setReason] = useState('');
  const [more, setMore] = useState(false);
  const [command, setCommand] = useState(card.envelope.target);
  const { envelope } = card;
  const left = secondsLeft(envelope, now);
  const fresh = needsFreshPasskey(envelope);
  const scopable = !fresh && envelope.risk !== 'DANGEROUS';
  const isCommand = envelope.action === 'run';
  const riskKey = `risk${envelope.risk}` as 'riskSAFE' | 'riskMUTATING' | 'riskDANGEROUS';
  const riskLabel = ['SAFE', 'MUTATING', 'DANGEROUS'].includes(envelope.risk) ? t(riskKey) : envelope.risk;
  const id = envelope.request_id;

  const answer = async (value: Answer) => {
    setBusy(true);
    await model.answer(card, value);
    setBusy(false);
  };

  return (
    <article className={`card risk-${envelope.risk.toLowerCase()}`} aria-labelledby={`t-${id}`}>
      <h2 id={`t-${id}`}>{envelope.title}</h2>
      <p className={`risk risk-${envelope.risk.toLowerCase()}`}>
        <span aria-hidden="true">{envelope.risk === 'DANGEROUS' ? '⚠' : envelope.risk === 'MUTATING' ? '✎' : '•'}</span> {t('risk')}: {riskLabel}
      </p>
      <dl>
        <dt>{t('host')}</dt>
        <dd>{envelope.host}</dd>
        <dt>{t('folder')}</dt>
        <dd className="mono">{envelope.cwd}</dd>
        <dt>{t('willRun')}</dt>
        <dd className="mono block">{envelope.target}</dd>
        {envelope.reason && (
          <>
            <dt>{t('why')}</dt>
            <dd>{envelope.reason}</dd>
          </>
        )}
        <dt>{t('rollback')}</dt>
        <dd>{envelope.rollback ? t('yes') : t('no')}</dd>
      </dl>
      {fresh && <p className="muted">{t('needsPasskey')}</p>}
      <label htmlFor={`why-${id}`}>{t('denyReason')}</label>
      <input id={`why-${id}`} value={reason} onChange={(event) => setReason(event.target.value)} maxLength={500} autoComplete="off" aria-describedby={`why-help-${id}`} />
      <p id={`why-help-${id}`} className="muted small">
        {t('denyReasonHelp')}
      </p>
      <p className="muted" aria-live="off">
        {left > 0 ? t('expiresIn', { s: left }) : t('expired')}
      </p>
      <div className="row">
        <button disabled={busy} onClick={() => void answer({ kind: 'deny', reason })}>
          {t('deny')}
        </button>
        <button className="primary" disabled={busy || left === 0} onClick={() => void answer({ kind: 'allow_once' })}>
          {t('allowOnce')}
        </button>
      </div>

      <button className="link" aria-expanded={more} aria-controls={`more-${id}`} onClick={() => setMore(!more)}>
        {t('moreOptions')} {more ? '▴' : '▾'}
      </button>
      {more && (
        <div id={`more-${id}`} className="more">
          {scopable && (
            <div>
              <button disabled={busy || left === 0} onClick={() => void answer({ kind: 'allow_scoped', minutes: 30 })}>
                {t('allowWhile', { m: 30 })}
              </button>
              <p className="muted small">{t('allowWhileHelp', { what: envelope.target.split(/\s+/).slice(0, 2).join(' ') })}</p>
            </div>
          )}
          {isCommand ? (
            <div>
              <label htmlFor={`cmd-${id}`}>{t('editCommand')}</label>
              <textarea id={`cmd-${id}`} value={command} onChange={(event) => setCommand(event.target.value)} rows={3} spellCheck={false} maxLength={2000} className="mono" />
              <button disabled={busy || left === 0 || !command.trim() || command.trim() === envelope.target} onClick={() => void answer({ kind: 'edit', command })}>
                {t('editSend')}
              </button>
              <p className="muted small">{t('editHelp')}</p>
            </div>
          ) : (
            <p className="muted small">{t('nothingToEdit')}</p>
          )}
          <button disabled={busy} onClick={() => void answer({ kind: 'deny_pause' })}>
            {t('denyPause')}
          </button>
        </div>
      )}
    </article>
  );
}
