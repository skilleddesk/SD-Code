import { useEffect, useState } from 'react';

import type { TelemetryView } from '../../../protocol/types';
import { strings } from '../strings';
import { clearTelemetry, loadTelemetry, sendTelemetry, setTelemetry } from '../store/telemetry';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * The consent screen for opt-in telemetry (Phase 4.2). Off by default. It lists the only things that can ever be counted, shows the exact
 * payload a send would post and what is stored on this computer, and says plainly that SDC has no server yet: until the person sets an
 * address, "on" only counts here.
 */
export function TelemetryCard() {
  const t = strings.telemetry;
  const [view, setView] = useState<TelemetryView | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [endpoint, setEndpoint] = useState('');

  useEffect(() => {
    void loadTelemetry().then((loaded) => {
      setView(loaded);
      setEndpoint(loaded?.endpoint ?? '');
    });
  }, []);

  if (view === null) {
    return null;
  }

  return (
    <section className="mt-[18px] rounded-md border border-border-subtle bg-bg-raised px-[14px] py-[12px]" data-telemetry="" data-opt-in={view.optIn}>
      <div className="flex items-center gap-[10px]">
        <div className="text-[12.5px] font-semibold text-text-primary">{t.title}</div>
        <label className="ml-auto flex items-center gap-[6px] text-[12px] text-text-secondary">
          <input
            type="checkbox"
            role="switch"
            checked={view.optIn}
            aria-label={t.title}
            onChange={(event) =>
              void setTelemetry(event.target.checked, endpoint).then((next) => {
                setView(next);
                setNote(null);
              })
            }
          />
          {view.optIn ? t.on : t.off}
        </label>
      </div>
      <p className="m-0 mt-[6px] text-[11.5px] leading-[1.55] text-text-muted">{t.promise}</p>
      <p className="m-0 mt-[4px] text-[11.5px] leading-[1.55] text-text-muted">{t.never}</p>
      <p className="m-0 mt-[4px] text-[11.5px] leading-[1.55] text-text-muted">{t.counted(view.eventNames.join(', '))}</p>

      <div className="mt-[8px] text-[11px] font-semibold uppercase tracking-[.08em] text-text-muted">{t.wouldSend}</div>
      <pre className="m-0 mt-[4px] max-h-[140px] overflow-auto rounded-sm bg-bg-input p-[8px] font-mono text-[10.5px] text-text-secondary" data-telemetry-payload="">
        {JSON.stringify(view.wouldSend, null, 2)}
      </pre>

      <div className="mt-[8px] text-[11px] font-semibold uppercase tracking-[.08em] text-text-muted">{t.local(view.events.length)}</div>
      {view.events.length === 0 ? (
        <p className="m-0 mt-[2px] text-[11.5px] text-text-muted">{view.optIn ? t.noneYet : t.nothingKept}</p>
      ) : (
        <ul className="m-0 mt-[4px] max-h-[110px] list-none overflow-auto p-0 font-mono text-[10.5px] text-text-secondary">
          {view.events.slice(-30).map((event, index) => (
            <li key={`${event.day}-${event.event}-${index}`}>{`${event.day} · ${event.event} · v${event.version} · ${event.os}`}</li>
          ))}
        </ul>
      )}

      {view.optIn ? (
        <div className="mt-[10px] flex flex-col gap-[6px]">
          <label className="flex items-center gap-[8px] text-[11.5px] text-text-secondary">
            {t.endpoint}
            <input
              className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[8px] py-[4px] font-mono text-[11px] text-text-primary"
              placeholder="https://"
              value={endpoint}
              spellCheck={false}
              onChange={(event) => setEndpoint(event.target.value)}
              onBlur={() => void setTelemetry(true, endpoint).then(setView)}
            />
          </label>
          <p className="m-0 text-[11px] text-text-faint">{t.noServer}</p>
          <div className="flex gap-[8px]">
            <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY} onClick={() => void sendTelemetry().then(setNote)} data-action="telemetry-send">
              {t.send}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void clearTelemetry().then(() => loadTelemetry().then(setView))}>
              {t.clear}
            </button>
          </div>
          {note === null ? null : (
            <p className="m-0 text-[11.5px] text-text-secondary" role="status">
              {note}
            </p>
          )}
        </div>
      ) : (
        <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' mt-[8px]'} onClick={() => void setTelemetry(true, endpoint).then(setView)}>
          {t.turnOn}
        </button>
      )}
    </section>
  );
}
