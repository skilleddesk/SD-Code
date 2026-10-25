import { useEffect, useState } from 'react';
import { Cpu, LoaderCircle, Plus, RefreshCw, Trash2 } from 'lucide-react';

import type { LocalServer } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { refreshCatalog } from '../store/intents';
import { refreshProviders } from '../store/tools';
import { toast } from '../store/toast';
import { rescanLocalRuntimes, useLocalRuntimes } from '../store/localRuntimes';
import { useProviderStore } from '../store/providers';
import { BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * Local model servers that are not Ollama (0.22): LM Studio, llama.cpp / llamafile / LocalAI, vLLM, SGLang, Jan,
 * KoboldCpp, text-generation-webui, GPT4All - anything that speaks OpenAI's API. SDC looks at the ports they use
 * on this computer by itself, and any address (another computer on the network, a custom port) can be added.
 * No key is needed; one can be given for a server started with one.
 */
export function LocalServers() {
  const words = strings.hub.localServers;
  const providers = useProviderStore((state) => state.providers);
  const connected = providers.filter((provider) => provider.id.startsWith('local-'));
  const [found, setFound] = useState<LocalServer[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [url, setUrl] = useState('');
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const runtimes = useLocalRuntimes((state) => state.scan);
  const looking = useLocalRuntimes((state) => state.scanning);

  const scan = (): void => {
    setScanning(true);
    void sdcpCall('local.discover', {}).then(
      ({ servers }) => {
        setFound(servers);
        setScanning(false);
      },
      () => {
        setFound([]);
        setScanning(false);
      },
    );
  };

  useEffect(scan, []);
  /* Opening the hub's Local tab looks again at once (Phase 2). */
  useEffect(() => {
    void rescanLocalRuntimes();
  }, []);

  const add = (address: string, label?: string, withKey?: string): void => {
    setBusy(address);
    void sdcpCall('local.add', { url: address, ...(label ? { label } : {}), ...(withKey ? { key: withKey } : {}) }).then(
      (added) => {
        setBusy(null);
        setUrl('');
        setKey('');
        toast(words.added(added.label, added.models.length));
        refreshProviders();
        void refreshCatalog();
        scan();
      },
      (reason: unknown) => {
        setBusy(null);
        toast(isSdcpError(reason) ? reason.message : String(reason));
      },
    );
  };

  const remove = (id: string): void => {
    void sdcpCall('local.remove', { id }).then(() => {
      refreshProviders();
      void refreshCatalog();
      scan();
    });
  };

  const r = strings.hub.runtimes;
  const ollama = runtimes?.runtimes.find((runtime) => runtime.kind === 'ollama');

  return (
    <>
      <section className="mt-[18px] flex flex-col gap-[8px]" data-local-runtimes>
        <div className="flex items-center gap-[8px]">
          <Cpu size={13} className="text-accent" aria-hidden="true" />
          <h3 className="text-[12.5px] font-semibold text-text-primary">{r.title}</h3>
          {looking ? <LoaderCircle size={11} className="animate-spin text-text-muted motion-reduce:animate-none" aria-label={r.scanning} /> : null}
        </div>
        {runtimes === null ? null : runtimes.none ? (
          <p className="text-[11.5px] leading-[1.55] text-text-muted" data-local-none>
            {r.none} {r.installHint}
          </p>
        ) : null}
        {ollama === undefined ? null : ollama.models.length === 0 ? (
          <p className="text-[11.5px] leading-[1.55] text-text-muted" data-local-ollama-empty>
            {ollama.state === 'silent' ? r.silent : r.ollamaEmpty}
          </p>
        ) : (
          <div className="rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[8px]" data-local-ollama>
            <div className="text-[12px] font-medium text-text-primary">{r.ollamaModels(ollama.models.length)}</div>
            <ul className="mt-[4px] flex flex-col gap-[2px]">
              {ollama.models.map((model) => (
                <li key={model.id} className="flex items-center gap-[8px] font-mono text-[11px] text-text-secondary">
                  <span className="min-w-0 flex-1 truncate">{model.id}</span>
                  <span className="rounded-full bg-bg-hover px-[7px] py-[1px] text-[10px] text-text-muted">{r.local}</span>
                  <span className="text-[10.5px] text-text-faint">{model.probe ? `${model.probe.class}` : r.measuring}</span>
                  {model.probe?.advice ? (
                    <span className="max-w-[46%] truncate font-sans text-[10.5px] text-text-muted" title={strings.prompt.model.quantAdvice(model.probe.advice)} data-local-advice={model.id}>
                      {strings.prompt.model.quantAdvice(model.probe.advice)}
                    </span>
                  ) : null}
                  {model.profileLine ? (
                    <span className="max-w-[46%] truncate font-sans text-[10.5px] text-text-faint" title={model.profileLine} data-local-profile={model.id}>
                      {model.profileLine}
                    </span>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>
      <section className="mt-[18px] flex flex-col gap-[10px]" data-local-servers>
      <div className="flex items-center gap-[8px]">
        <Cpu size={13} className="text-accent" aria-hidden="true" />
        <h3 className="text-[12.5px] font-semibold text-text-primary">{words.title}</h3>
        <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' ml-auto'} onClick={scan} disabled={scanning} data-action="local-scan">
          {scanning ? <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <RefreshCw size={11} aria-hidden="true" />}
          {words.scan}
        </button>
      </div>
      <p className="text-[11.5px] leading-[1.55] text-text-muted">{words.help}</p>

      {connected.length === 0 ? null : (
        <ul className="flex flex-col gap-[6px]">
          {connected.map((provider) => (
            <li key={provider.id} className="flex items-center gap-[10px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[8px]" data-local-connected={provider.id}>
              <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-state-success" aria-hidden="true" />
              <span className="min-w-0 flex-1">
                <span className="block text-[12px] font-medium text-text-primary">{provider.name}</span>
                <span className="block truncate font-mono text-[10.5px] text-text-muted">{provider.detail}</span>
              </span>
              <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => remove(provider.id)} aria-label={words.remove(provider.name)}>
                <Trash2 size={11} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {found === null ? null : found.filter((server) => !server.connected).length === 0 ? (
        <p className="text-[11px] text-text-faint">{scanning ? words.scanning : words.noneFound}</p>
      ) : (
        <ul className="flex flex-col gap-[6px]">
          {found
            .filter((server) => !server.connected)
            .map((server) => (
              <li key={server.id} className="flex items-center gap-[10px] rounded-md border border-accent/30 bg-accent-subtle px-[12px] py-[8px]" data-local-found={server.id}>
                <span className="min-w-0 flex-1">
                  <span className="block text-[12px] font-medium text-text-primary">{words.found(server.label)}</span>
                  <span className="block truncate font-mono text-[10.5px] text-text-muted">
                    {server.base} · {server.note ?? server.models.map((model) => model.id).slice(0, 3).join(', ')}
                  </span>
                </span>
                <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY} disabled={busy !== null} onClick={() => add(server.base, server.label)} data-action="local-connect-found">
                  {busy === server.base ? <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
                  {words.connect}
                </button>
              </li>
            ))}
        </ul>
      )}

      <form
        className="flex flex-col gap-[6px] rounded-md border border-border-subtle bg-bg-raised p-[10px]"
        onSubmit={(event) => {
          event.preventDefault();

          if (url.trim() !== '') add(url.trim(), undefined, key.trim() || undefined);
        }}
      >
        <label className="text-[11px] font-medium text-text-secondary" htmlFor="localServerUrl">
          {words.addTitle}
        </label>
        <div className="flex gap-[6px]">
          <input
            id="localServerUrl"
            className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[9px] py-[5px] font-mono text-[11.5px] text-text-primary placeholder:text-text-faint"
            placeholder="http://127.0.0.1:8080/v1"
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            spellCheck={false}
          />
          <input
            className="w-[130px] rounded-md border border-border-default bg-bg-input px-[9px] py-[5px] font-mono text-[11.5px] text-text-primary placeholder:text-text-faint"
            placeholder={words.keyPlaceholder}
            value={key}
            onChange={(event) => setKey(event.target.value)}
            type="password"
            autoComplete="off"
            aria-label={words.keyPlaceholder}
          />
          <button type="submit" className={BTN_SM + ' ' + BTN_PRIMARY} disabled={busy !== null || url.trim() === ''} data-action="local-add">
            <Plus size={11} aria-hidden="true" />
            {words.add}
          </button>
        </div>
        <span className="text-[10.5px] text-text-faint">{words.addHelp}</span>
      </form>
      </section>
    </>
  );
}
