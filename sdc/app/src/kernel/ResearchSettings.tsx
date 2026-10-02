import { useEffect, useState } from 'react';

import type { ResearchStatus } from '../../../protocol/types';
import { strings } from '../strings';
import { useModelStore } from '../store/model';
import { researchStatus, setResearchKey, setResearchSetting } from '../store/research';
import { toast } from '../store/toast';
import { BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/** The local model the plan this tab was built for runs (docs/ANALYSIS-local-model-and-research.md). */
const SUGGESTED_LOCAL_MODEL = 'qwen3.5:9b';

const PROVIDERS = ['duckduckgo', 'searxng', 'tavily', 'brave', 'serper'] as const;

const FIELD =
  'h-[30px] rounded-md border border-border-default bg-bg-base px-[10px] text-[12.5px] text-text-primary focus:border-border-strong focus:outline-none';

/**
 * Settings → Research (0.16.1): the search service `/research` asks and its key (kept in the OS
 * keychain, shown masked), how far one question may go, whether a local model may use the web outside
 * `/research`, an optional model for the final answer, and how much context a local model loads.
 */
export function ResearchSettings() {
  const words = strings.research.settings;
  const [status, setStatus] = useState<ResearchStatus | null>(null);
  const [key, setKey] = useState('');
  const catalog = useModelStore((state) => state.catalog);

  useEffect(() => {
    void researchStatus().then(setStatus);
  }, []);

  if (status === null) {
    return (
      <section className="mb-[20px]">
        <h3 className="text-[14px] font-semibold text-text-primary">{words.title}</h3>
        <p className="mt-[2px] text-[12px] text-text-muted">{words.intro}</p>
      </section>
    );
  }

  const save = (name: string, value: string, next: Partial<ResearchStatus>): void => {
    void setResearchSetting(name, value).then((ok) => {
      if (ok) {
        setStatus((current) => (current === null ? current : { ...current, ...next }));
        toast(words.saved);
      }
    });
  };
  const keyed = status.keys.find((entry) => entry.provider === status.provider);
  /* A model that writes the final answer is a paid API model: subscriptions run their own research. */
  const apiModels = catalog.filter((model) => model.providerId !== 'ollama' && !['claude', 'openai', 'gemini'].includes(model.providerId));
  const synthesisValue = status.synthesis === null ? '' : `${status.synthesis.provider}|${status.synthesis.model}`;
  const number = (name: string, value: number, apply: (value: number) => Partial<ResearchStatus>) => (
    <input
      type="number"
      min={1}
      className={FIELD + ' w-[78px] font-mono'}
      defaultValue={value}
      onBlur={(event) => {
        const parsed = Number.parseInt(event.target.value, 10);

        if (Number.isFinite(parsed) && parsed > 0 && parsed !== value) {
          save(name, String(parsed), apply(parsed));
        }
      }}
    />
  );

  return (
    <section className="mb-[20px]">
      <h3 className="text-[14px] font-semibold text-text-primary">{words.title}</h3>
      <p className="mt-[2px] text-[12px] text-text-muted">{words.intro}</p>

      <label className="mt-[14px] block text-[12.5px] text-text-primary">
        {words.provider}
        <select
          className={FIELD + ' mt-[4px] block w-full'}
          value={status.provider}
          onChange={(event) => {
            const provider = event.target.value as ResearchStatus['provider'];

            save('research.searchProvider', provider, { provider });
          }}
        >
          {PROVIDERS.map((id) => (
            <option key={id} value={id}>
              {words.providers[id]}
            </option>
          ))}
        </select>
      </label>

      {status.provider === 'searxng' ? (
        <label className="mt-[10px] block text-[12.5px] text-text-primary">
          {words.searxngUrl}
          <input
            className={FIELD + ' mt-[4px] block w-full font-mono'}
            defaultValue={status.searxngUrl}
            placeholder="https://search.example.org"
            onBlur={(event) => {
              if (event.target.value.trim() !== status.searxngUrl) {
                save('research.searxngUrl', event.target.value.trim(), { searxngUrl: event.target.value.trim() });
              }
            }}
          />
          <span className="mt-[2px] block text-[11.5px] text-text-muted">{words.searxngHint}</span>
        </label>
      ) : null}

      {keyed !== undefined ? (
        <div className="mt-[10px] text-[12.5px] text-text-primary">
          {words.key}
          <span className="ml-[8px] text-[11.5px] text-text-muted">{keyed.hasKey && keyed.masked !== null ? words.keySaved(keyed.masked) : words.keyNone}</span>
          <div className="mt-[4px] flex flex-wrap items-center gap-[8px]">
            <input
              type="password"
              autoComplete="off"
              className={FIELD + ' min-w-[220px] flex-1 font-mono'}
              value={key}
              aria-label={words.key}
              onChange={(event) => setKey(event.target.value)}
            />
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_SECONDARY}
              disabled={key.trim() === ''}
              onClick={() =>
                void setResearchKey(keyed.provider, key).then((next) => {
                  if (next !== null) {
                    setStatus(next);
                    setKey('');
                  }
                })
              }
            >
              {words.saveKey}
            </button>
            {keyed.hasKey ? (
              <button
                type="button"
                className={BTN_SM + ' ' + BTN_SECONDARY}
                onClick={() => void setResearchKey(keyed.provider, '').then((next) => next !== null && setStatus(next))}
              >
                {words.removeKey}
              </button>
            ) : null}
          </div>
        </div>
      ) : null}

      <div className="mt-[14px] text-[12.5px] text-text-primary">
        {words.limits}
        <div className="mt-[4px] flex flex-wrap items-center gap-[14px] text-[12px] text-text-secondary">
          <label className="flex items-center gap-[6px]">
            {words.maxSearches} {number('research.maxSearches', status.limits.maxSearches, (value) => ({ limits: { ...status.limits, maxSearches: value } }))}
          </label>
          <label className="flex items-center gap-[6px]">
            {words.maxPages} {number('research.maxPages', status.limits.maxPages, (value) => ({ limits: { ...status.limits, maxPages: value } }))}
          </label>
          <label className="flex items-center gap-[6px]">
            {words.maxMinutes} {number('research.maxMinutes', status.limits.maxMinutes, (value) => ({ limits: { ...status.limits, maxMinutes: value } }))}
          </label>
        </div>
      </div>

      <label className="mt-[14px] flex items-start gap-[10px]">
        <input
          type="checkbox"
          className="mt-[3px] accent-[var(--accent)]"
          checked={status.localWebOnly}
          onChange={(event) => save('research.localWebOnly', event.target.checked ? 'on' : 'off', { localWebOnly: event.target.checked })}
        />
        <span>
          <span className="block text-[12.5px] text-text-primary">{words.localWebOnly}</span>
          <span className="block text-[11.5px] text-text-muted">{words.localWebOnlyHint}</span>
        </span>
      </label>

      <label className="mt-[14px] block text-[12.5px] text-text-primary">
        {words.synthesis}
        <select
          className={FIELD + ' mt-[4px] block w-full'}
          value={synthesisValue}
          onChange={(event) => {
            const [provider = '', model = ''] = event.target.value.split('|');
            const synthesis = provider === '' ? null : { provider, model };

            void setResearchSetting('research.synthesisProvider', provider).then((ok) => {
              if (ok) {
                save('research.synthesisModel', model, { synthesis });
              }
            });
          }}
        >
          <option value="">{words.synthesisNone}</option>
          {apiModels.map((model) => (
            <option key={`${model.providerId}|${model.id}`} value={`${model.providerId}|${model.id}`}>
              {model.providerLabel} · {model.name === '' ? model.id : model.name}
              {model.cost === '' ? '' : ` · ${model.cost}`}
            </option>
          ))}
        </select>
        <span className="mt-[2px] block text-[11.5px] text-text-muted">{words.synthesisHint}</span>
      </label>

      <label className="mt-[14px] block text-[12.5px] text-text-primary">
        {words.localContext}
        <span className="mt-[4px] flex items-center gap-[8px]">
          {number('ollama.contextTokens', status.ollamaContext, (value) => ({ ollamaContext: value }))}
        </span>
        <span className="mt-[2px] block text-[11.5px] text-text-muted">{words.localContextHint}</span>
      </label>

      <p className="mt-[12px] text-[12px] text-text-secondary">{words.ollama(status.ollamaRunning, status.ollamaModels.length)}</p>
      {status.ollamaRunning && !status.ollamaModels.some((name) => name === SUGGESTED_LOCAL_MODEL || name.startsWith(`${SUGGESTED_LOCAL_MODEL}-`)) ? (
        <p className="mt-[2px] font-mono text-[11.5px] text-text-muted">{words.pull(SUGGESTED_LOCAL_MODEL)}</p>
      ) : null}
    </section>
  );
}
