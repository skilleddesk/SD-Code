import { ArrowDown, ArrowUp, Plus, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import type { RelaySettings, RotationGlobal, RotationRowInput, RotationView } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { refreshCatalog } from '../store/intents';
import { groupCatalog, modelMode, useModelStore } from '../store/model';
import { useAppStore } from '../store/store';
import { Badge } from '../panels/ui/Badge';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * Settings → Agents (Phase 3): the Rotation Chain, edited one entry at a time.
 *
 * The chain is only editable when the model you chose is a local one. With a cloud, API or CLI model chosen the tab shows the chain
 * read-only beside a plain explanation: that is Manual Pin - SDC never changes the model by itself. Entries are written to
 * `rotation.toml` by the daemon, which validates them; a file that cannot be read shows its reason and an empty chain.
 */
const inputClass = 'rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary disabled:opacity-60';

const split = (model: string): [string, string] => {
  const at = model.indexOf(':');

  return at < 0 ? ['', model] : [model.slice(0, at), model.slice(at + 1)];
};

export function AgentsTab() {
  const t = strings.agents;
  const providerId = useModelStore((state) => state.providerId);
  const rotationRows = useModelStore((state) => state.rotationRows);
  const catalog = useModelStore((state) => state.catalog);
  const providers = useAppStore((state) => state.providers);
  const [relay, setRelay] = useState<RelaySettings | null>(null);
  const mode = modelMode(providerId, rotationRows, relay?.auto ?? false);
  const editable = providerId === 'ollama';
  const [view, setView] = useState<RotationView | null>(null);
  const [rows, setRows] = useState<RotationRowInput[]>([]);
  const [global, setGlobal] = useState<RotationGlobal | null>(null);
  const [status, setStatus] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null);

  const load = (next: RotationView): void => {
    setView(next);
    setRows(next.rows.map((row) => ({ model: row.model, enabled: row.enabled, allowedSteps: row.allowedSteps, maxCostPerStep: row.maxCostPerStep, maxRepairRetries: row.maxRepairRetries })));
    setGlobal(next.global);
    setRelay(next.relay);
  };

  useEffect(() => {
    void sdcpCall('rotation.get', {}).then(load, () => undefined);
  }, []);

  /* Cloud, API and CLI models of the connected providers (a local model is never a chain entry: it is the one being helped). */
  const groups = useMemo(() => groupCatalog(catalog, providers).groups.filter((group) => group.providerId !== 'ollama' && !group.providerId.startsWith('local-')), [catalog, providers]);

  const update = (index: number, patch: Partial<RotationRowInput>): void => {
    setRows((current) => current.map((row, at) => (at === index ? { ...row, ...patch } : row)));
    setStatus(null);
  };

  const move = (index: number, by: -1 | 1): void => {
    setRows((current) => {
      const next = [...current];
      const target = index + by;

      if (target < 0 || target >= next.length) {
        return current;
      }

      [next[index], next[target]] = [next[target] as RotationRowInput, next[index] as RotationRowInput];

      return next;
    });
    setStatus(null);
  };

  const add = (): void => {
    const group = groups[0];
    const model = group?.models[0];

    if (group === undefined || model === undefined) {
      return;
    }

    setRows((current) => [...current, { model: `${group.providerId}:${model.id}`, enabled: true, allowedSteps: [], maxCostPerStep: 0.05, maxRepairRetries: 1 }]);
    setStatus(null);
  };

  const save = (): void => {
    void sdcpCall('rotation.set', { rows, ...(global === null ? {} : { global }), ...(relay === null ? {} : { relay }) }).then(
      (saved) => {
        load(saved);
        setStatus({ tone: 'ok', text: t.saved });
        /* the mode badge beside the model button follows the chain */
        void refreshCatalog();
      },
      (reason: unknown) => setStatus({ tone: 'error', text: isSdcpError(reason) ? reason.message : t.failed }),
    );
  };

  const hintOf = (model: string): string => {
    const row = view?.rows.find((candidate) => candidate.model === model);

    if (row === undefined) {
      return '';
    }

    if (!row.cost.known) {
      return t.costUnknown;
    }

    return row.cost.perStep === 0 ? t.costFree : t.costPerStep((row.cost.perStep ?? 0).toFixed(4));
  };

  const badge = mode === 'pinned' ? t.modePinned : mode === 'silent-relay' ? t.modeSilentRelay : mode === 'local-rotation' ? t.modeLocalRotation : t.modeLocalOnly;

  return (
    <div data-agents-tab="">
      <div className="mb-[14px]">
        <h2 className="text-[17px] font-semibold text-text-primary">{t.title}</h2>
        <p className="mt-[4px] text-[12.5px] text-text-muted">{t.desc}</p>
      </div>

      <div className="mb-[12px] flex items-start gap-[10px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px]" data-agents-mode={mode}>
        <Badge tone={mode === 'pinned' ? 'muted' : 'success'}>{badge}</Badge>
        <p className="m-0 text-[12px] leading-[1.6] text-text-secondary">
          {mode === 'pinned' ? t.pinnedExplain : mode === 'local-only' ? t.localOnlyExplain : mode === 'silent-relay' ? t.silentRelayExplain(rotationRows) : t.rotationExplain(rotationRows)}
        </p>
      </div>

      {view?.warning ? (
        <p className="mb-[10px] text-[12px] text-state-waiting" role="status">
          {t.broken(view.warning)}
        </p>
      ) : null}

      <div className="mb-[6px] text-[12.5px] font-medium text-text-primary">{t.chain}</div>
      {rows.length === 0 ? <p className="text-[12px] text-text-muted">{t.empty}</p> : null}

      <ol className="m-0 flex list-none flex-col gap-[8px] p-0">
        {rows.map((row, index) => {
          const [provider, model] = split(row.model);
          const group = groups.find((candidate) => candidate.providerId === provider);

          return (
            <li key={`${row.model}-${index}`} className="rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px]" data-agents-row={index} data-enabled={row.enabled}>
              <div className="flex flex-wrap items-center gap-[8px]">
                <span className="w-[64px] text-[11.5px] font-semibold text-text-muted">{t.position(index + 1)}</span>
                <select
                  className={inputClass}
                  aria-label={t.provider}
                  disabled={!editable}
                  value={provider}
                  onChange={(event) => {
                    const next = groups.find((candidate) => candidate.providerId === event.target.value);

                    update(index, { model: `${event.target.value}:${next?.models[0]?.id ?? ''}` });
                  }}
                >
                  {group === undefined ? <option value={provider}>{provider}</option> : null}
                  {groups.map((candidate) => (
                    <option key={candidate.providerId} value={candidate.providerId}>
                      {candidate.providerLabel}
                    </option>
                  ))}
                </select>
                <select className={inputClass} aria-label={t.model} disabled={!editable} value={model} onChange={(event) => update(index, { model: `${provider}:${event.target.value}` })}>
                  {group?.models.some((candidate) => candidate.id === model) ? null : <option value={model}>{model}</option>}
                  {group?.models.map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>
                      {candidate.name === '' ? candidate.id : candidate.name}
                    </option>
                  ))}
                </select>
                <label className="ml-auto flex items-center gap-[6px] text-[12px] text-text-secondary">
                  <input type="checkbox" role="switch" checked={row.enabled} disabled={!editable} onChange={(event) => update(index, { enabled: event.target.checked })} />
                  {t.enabled}
                </label>
                <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} aria-label={t.up} disabled={!editable || index === 0} onClick={() => move(index, -1)}>
                  <ArrowUp size={11} aria-hidden="true" />
                </button>
                <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} aria-label={t.down} disabled={!editable || index === rows.length - 1} onClick={() => move(index, 1)}>
                  <ArrowDown size={11} aria-hidden="true" />
                </button>
                <button
                  type="button"
                  className={BTN_SM + ' ' + BTN_SECONDARY}
                  aria-label={t.remove(row.model)}
                  disabled={!editable}
                  onClick={() => {
                    setRows((current) => current.filter((_, at) => at !== index));
                    setStatus(null);
                  }}
                >
                  <Trash2 size={11} aria-hidden="true" />
                </button>
              </div>
              <p className="mb-0 mt-[4px] text-[11.5px] text-text-muted" data-agents-cost>
                {hintOf(row.model)}
              </p>
            </li>
          );
        })}
      </ol>

      {editable && groups.length === 0 ? <p className="mt-[8px] text-[12px] text-text-muted">{t.noProviders}</p> : null}

      <div className="mt-[10px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} disabled={!editable || groups.length === 0 || rows.length >= 12} onClick={add} data-action="agents-add">
          <Plus size={12} aria-hidden="true" />
          {t.add}
        </button>
      </div>

      {relay === null ? null : (
        <div className="mt-[16px] flex flex-col gap-[8px] border-t border-border-subtle pt-[12px]" data-agents-relay="">
          <div className="text-[12.5px] font-medium text-text-primary">{t.relay.title}</div>
          <label className="flex items-center gap-[8px] text-[12.5px] text-text-primary">
            <input
              type="checkbox"
              role="switch"
              checked={relay.auto}
              disabled={!editable}
              onChange={(event) => {
                setRelay({ ...relay, auto: event.target.checked });
                setStatus(null);
              }}
            />
            {t.relay.auto}
          </label>
          <p className="m-0 text-[11.5px] leading-[1.5] text-text-muted" data-agents-consent="">
            {t.relay.consent}
          </p>
          <div className="flex flex-wrap items-center gap-[14px] text-[12.5px] text-text-primary">
            <label className="flex items-center gap-[8px]" htmlFor="agents-relay-steps">
              {t.relay.maxSteps}
              <input
                id="agents-relay-steps"
                type="number"
                min={1}
                max={50}
                className={inputClass + ' w-[70px]'}
                disabled={!editable || !relay.auto}
                value={relay.maxCloudStepsPerTurn}
                onChange={(event) => {
                  setRelay({ ...relay, maxCloudStepsPerTurn: Math.max(1, Math.min(50, Number(event.target.value) || 1)) });
                  setStatus(null);
                }}
              />
            </label>
            <label className="flex items-center gap-[8px]" htmlFor="agents-relay-usd">
              {t.relay.maxUsd}
              <input
                id="agents-relay-usd"
                type="number"
                min={0}
                max={100}
                step={0.05}
                className={inputClass + ' w-[80px]'}
                disabled={!editable || !relay.auto}
                value={relay.maxUsdPerTurn}
                onChange={(event) => {
                  setRelay({ ...relay, maxUsdPerTurn: Math.max(0, Math.min(100, Number(event.target.value) || 0)) });
                  setStatus(null);
                }}
              />
            </label>
            <label className="flex items-center gap-[8px]">
              <input
                type="checkbox"
                role="switch"
                checked={relay.showChip}
                disabled={!editable}
                onChange={(event) => {
                  setRelay({ ...relay, showChip: event.target.checked });
                  setStatus(null);
                }}
              />
              {t.relay.showChip}
            </label>
          </div>
          <p className="m-0 text-[11.5px] text-text-muted">{t.relay.limits}</p>
        </div>
      )}

      {global === null ? null : (
        <div className="mt-[16px] flex flex-col gap-[10px] border-t border-border-subtle pt-[12px]">
          <label className="flex items-center gap-[10px] text-[12.5px] text-text-primary" htmlFor="agents-offload">
            {t.offloadLimit}
            <input
              id="agents-offload"
              type="number"
              min={0}
              max={1000}
              className={inputClass + ' w-[80px]'}
              disabled={!editable}
              value={global.offloadLimit}
              onChange={(event) => {
                setGlobal({ ...global, offloadLimit: Math.max(0, Math.min(1000, Number(event.target.value) || 0)) });
                setStatus(null);
              }}
            />
          </label>
          <p className="m-0 text-[11.5px] text-text-muted">{t.offloadHelp}</p>
          <label className="flex items-center gap-[8px] text-[12.5px] text-text-primary">
            <input
              type="checkbox"
              role="switch"
              checked={global.askBeforeCloud}
              disabled={!editable}
              onChange={(event) => {
                setGlobal({ ...global, askBeforeCloud: event.target.checked });
                setStatus(null);
              }}
            />
            {t.askBeforeCloud}
          </label>
        </div>
      )}

      <div className="mt-[16px] flex items-center gap-[10px]">
        <button type="button" className={BTN + ' ' + BTN_PRIMARY} disabled={!editable} onClick={save} data-action="agents-save">
          {t.save}
        </button>
        {status === null ? null : (
          <span className={status.tone === 'ok' ? 'text-[12px] text-state-success' : 'text-[12px] text-state-error'} role={status.tone === 'ok' ? 'status' : 'alert'}>
            {status.text}
          </span>
        )}
      </div>
      {view === null ? null : <p className="mt-[10px] text-[11px] text-text-faint">{t.file(view.path)}</p>}
    </div>
  );
}
