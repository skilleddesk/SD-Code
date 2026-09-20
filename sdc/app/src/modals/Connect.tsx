import { useEffect, useRef, useState } from 'react';
import { Check, ClipboardCopy, ExternalLink, RefreshCw, Terminal, TriangleAlert } from 'lucide-react';

import { strings } from '../strings';
import {
  cancelCliLogin,
  chooseModel,
  connectApiKey,
  loadModels,
  pollCliLogin,
  startCliLogin,
  submitCliLoginCode,
  type CliLoginView,
  type ModelsView,
} from '../store/intents';
import { useOverlayStore } from '../store/overlays';
import { useProviderStore } from '../store/providers';
import { toast } from '../store/toast';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * `#connectBd` - the two flows that get a user working (spec section 9.10).
 *
 * It is its own surface because these are the two things a person has to *do* before SDC can help
 * them, and neither fits in a card:
 *
 *  * **Sign in.** The daemon starts the CLI's own login (`cli.login`), which prints a URL. The URL is
 *    shown here with a copy button and a box for the code the browser gives back. SDC never sees the
 *    credential - the CLI writes it - and the CLI's own output is shown underneath, so a sign-in that
 *    goes sideways is visible instead of mysterious.
 *  * **API key and model.** The key goes to the keychain, and the model list is the daemon's
 *    catalogue: `live` rows came from the provider just now, `cached` rows are its last answer, and
 *    `bundled` rows shipped with this build. Refresh asks again; a refresh that could not reach the
 *    provider says so and keeps the list.
 *
 * The daemon does the work in both cases. This component polls and renders, and keeps no state that
 * the daemon or the event log already has.
 */
export function Connect() {
  const open = useOverlayStore((state) => state.connectOpen);
  const providerId = useOverlayStore((state) => state.connectProviderId);
  const mode = useOverlayStore((state) => state.connectMode);
  const close = useOverlayStore((state) => state.closeConnect);
  const providers = useProviderStore((state) => state.providers);

  const provider = providers.find((candidate) => candidate.id === providerId);

  const [key, setKey] = useState('');
  const [login, setLogin] = useState<CliLoginView | null>(null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [models, setModels] = useState<ModelsView | null>(null);

  /*
   * The sign-in starts itself, and that is the 0.6.1 change to this modal.
   *
   * Clicking a subscription card means "use my Claude / ChatGPT / Gemini plan", and a person who has
   * just clicked it should not then have to find a second button called `Sign in` to say the same
   * thing again. Opening this modal in `login` mode starts the CLI's own login immediately; the CLI is
   * the only party that can authenticate, so what the user gets is the provider's real page and a code
   * to paste back - no credential ever passes through SDC.
   *
   * Once per open, tracked by provider id: a login that failed, or a user who cancelled, must not be
   * restarted by a re-render. Closing the modal clears the note and the next open tries again; the
   * `Try again` button is the retry inside one open.
   */
  const startedFor = useRef<string | null>(null);
  const announced = useRef<string | null>(null);

  /*
   * A finished sign-in says so out loud, once.
   *
   * The card flips because the daemon pushes a `ProviderStatus` the moment its `cli.login.status`
   * sees the CLI's success line - and this is the sentence next to it, because "it worked" is the one
   * thing a person cannot check for themselves: the credential is in the CLI's own store. Guarded by
   * a ref keyed on the login id, since the modal polls once a second and would otherwise repeat it.
   */
  useEffect(() => {
    if (login === null || !login.authenticated || announced.current === login.loginId) {
      return;
    }

    announced.current = login.loginId;
    toast(strings.connect.signedInToast(login.providerLabel));
  }, [login]);

  useEffect(() => {
    if (!open) {
      startedFor.current = null;
      return;
    }

    if (mode !== 'login' || providerId === null || providerId === undefined || startedFor.current === providerId) {
      return;
    }

    startedFor.current = providerId;
    setBusy(true);

    void startCliLogin(providerId).then((view) => {
      setBusy(false);
      setLogin(view);
    });
  }, [open, mode, providerId]);

  /* One poll per second while a sign-in is in flight: the URL, the CLI's tail, and the moment it says
     it is done. The poll stops the instant the CLI has spoken, so a finished login costs nothing. */
  useEffect(() => {
    if (login === null || login.authenticated || login.state === 'exited' || login.state === 'failed') {
      return undefined;
    }

    const timer = window.setInterval(() => {
      void pollCliLogin(login.loginId).then((next) => {
        if (next !== null) {
          setLogin(next);
        }
      });
    }, 1000);

    return () => window.clearInterval(timer);
  }, [login]);

  /* Opening on a provider loads its models once, so the list is never empty for no reason. */
  useEffect(() => {
    if (!open || mode !== 'api' || providerId === undefined) {
      return;
    }

    void loadModels(providerId, false).then(setModels);
  }, [open, mode, providerId]);

  const name = provider?.name ?? providerId ?? '';

  /*
   * Whether the CLI is still waiting for a code.
   *
   * The paste boxes are only drawn while that is true. A sign-in that has stopped - accepted, failed,
   * cancelled - keeps no field: the screenshot that started this had a code box and a `Submit code`
   * under a CLI that had already said `Login failed ... 400`, which invites a person to paste into
   * something that cannot answer.
   */
  const waitingForCode =
    login !== null &&
    !login.authenticated &&
    login.url !== null &&
    (login.state === 'waiting_for_code' || login.state === 'waiting_for_url');

  const copy = (url: string): void => {
    void navigator.clipboard?.writeText(url).then(() => toast(strings.connect.copied));
  };

  const signIn = (): void => {
    setBusy(true);
    void startCliLogin(providerId ?? '').then((view) => {
      setBusy(false);
      setLogin(view);
    });
  };

  const submit = (): void => {
    if (login === null || code.trim() === '') {
      return;
    }

    setBusy(true);
    void submitCliLoginCode(login.loginId, code).then((view) => {
      setBusy(false);
      setCode('');
      setLogin(view);
    });
  };

  const load = (refresh: boolean): void => {
    setBusy(true);
    void loadModels(providerId ?? null, refresh).then((view) => {
      setBusy(false);
      setModels(view);
    });
  };

  const use = (modelId: string): void => {
    void chooseModel(modelId, providerId ?? '').then((ok) => {
      if (ok) {
        setModels((current) => (current === null ? current : { ...current, selected: { modelId, providerId: providerId ?? null } }));
        toast(`${modelId} · ${strings.connect.selected}`);
      }
    });
  };

  return (
    <Modal
      open={open}
      label={`${strings.connect.title} · ${name}`}
      onClose={() => {
        if (login !== null && !login.authenticated && login.state !== 'exited') {
          void cancelCliLogin(login.loginId);
        }

        setLogin(null);
        close();
      }}
      center
      className="connect-dlg w-[min(560px,96vw)]"
    >
      <div className="flex flex-col gap-[14px] p-[16px]" data-connect={mode} data-connect-provider={providerId ?? ''}>
        <div>
          <h2 className="text-[14px] font-semibold text-text-primary">
            {mode === 'login' ? strings.connect.loginTitle : strings.connect.apiTitle}
          </h2>
          <p className="mt-[4px] text-[12px] text-text-secondary">
            {mode === 'login' ? strings.connect.loginBody : strings.connect.apiBody}
          </p>
        </div>

        {mode === 'login' ? (
          <>
            {login === null ? (
              <div className="flex items-center gap-[8px]">
                <button type="button" className={BTN_PRIMARY} onClick={signIn} disabled={busy} id="connectSignIn">
                  <Terminal size={13} /> {strings.connect.signIn}
                </button>
                <span className="text-[11.5px] text-text-muted">{name}</span>
              </div>
            ) : null}

            {login !== null ? (
              <>
                <div className="rounded-md border border-border-subtle bg-bg-raised p-[10px]">
                  <div className="flex items-start gap-[6px] text-[11.5px] text-text-secondary">
                    {login.authenticated ? (
                      <>
                        <Check size={13} className="mt-[1px] shrink-0 text-state-success" />
                        <span>
                          <span className="font-medium text-state-success">{strings.connect.authenticated}</span>
                          {' · '}
                          {strings.connect.signedInNote}
                        </span>
                      </>
                    ) : login.state === 'failed' || login.state === 'cancelled' ? (
                      /* The CLI stopped without a credential: say that, rather than leaving the
                         `Waiting for the CLI…` line of a login that is over. */
                      <>
                        <TriangleAlert size={13} className="mt-[1px] shrink-0 text-state-error" />
                        <span>{strings.connect.failed}</span>
                      </>
                    ) : login.state === 'exited' ? (
                      <>
                        <TriangleAlert size={13} className="mt-[1px] shrink-0 text-state-waiting" />
                        <span>{strings.connect.finished}</span>
                      </>
                    ) : (
                      <>
                        <RefreshCw size={13} className="mt-[1px] shrink-0 animate-spin text-state-waiting" />
                        <span>
                          {login.state === 'waiting_for_code' ? strings.connect.waitingForCode : strings.connect.waiting}
                        </span>
                      </>
                    )}
                  </div>

                  {waitingForCode ? (
                    <>
                      <div className="mt-[10px] text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted">
                        {strings.connect.pageLabel}
                      </div>
                      <div className="mt-[4px] flex items-center gap-[6px]">
                        <input
                          readOnly
                          id="connectUrl"
                          aria-label={strings.connect.copyLink}
                          value={login.url ?? ''}
                          className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[8px] py-[6px] font-mono text-[11.5px] text-text-primary"
                        />
                        <button type="button" className={BTN_SECONDARY} onClick={() => copy(login.url ?? '')} id="connectCopy">
                          <ClipboardCopy size={13} />
                        </button>
                        <a className={BTN_SECONDARY} href={login.url ?? '#'} target="_blank" rel="noreferrer noopener">
                          <ExternalLink size={13} />
                        </a>
                      </div>

                      <div className="mt-[10px] text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted">
                        {strings.connect.codeLabel}
                      </div>
                      <div className="mt-[4px] flex items-center gap-[6px]">
                        <input
                          id="connectCode"
                          aria-label={strings.connect.codeLabel}
                          placeholder={strings.connect.codePlaceholder}
                          value={code}
                          onChange={(event) => setCode(event.target.value)}
                          onKeyDown={(event) => {
                            if (event.key === 'Enter' && code.trim() !== '') {
                              submit();
                            }
                          }}
                          className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[8px] py-[6px] font-mono text-[11.5px] text-text-primary placeholder:text-text-muted"
                        />
                        {/* Disabled until there is something to submit: a primary button over an empty
                            field is the design mistake that was reported, and it is also the one that
                            sends an empty line to a CLI that is waiting for a code. */}
                        <button
                          type="button"
                          className={BTN_PRIMARY}
                          onClick={submit}
                          disabled={busy || code.trim() === ''}
                          id="connectSubmit"
                        >
                          {strings.connect.submitCode}
                        </button>
                      </div>
                      <p className="mt-[6px] text-[11px] text-text-muted">{strings.connect.codeHint}</p>
                    </>
                  ) : null}

                  {login.note !== null ? (
                    <p className="mt-[8px] text-[11.5px] text-text-muted">{login.note}</p>
                  ) : null}

                  {/* The retry, for a sign-in that stopped: this is the one state that needs a
                      control of its own, because the first attempt was started by the click that
                      opened this modal. */}
                  {!login.authenticated &&
                  (login.state === 'failed' || login.state === 'cancelled' || login.state === 'exited') ? (
                    <button
                      type="button"
                      className={BTN_SECONDARY + ' mt-[8px]'}
                      onClick={signIn}
                      disabled={busy}
                      id="connectRetry"
                    >
                      <RefreshCw size={13} /> {strings.connect.tryAgain}
                    </button>
                  ) : null}
                </div>

                <div>
                  <div className="mb-[4px] text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted">
                    {strings.connect.outputTitle}
                  </div>
                  <pre
                    id="connectOutput"
                    className="max-h-[140px] overflow-auto rounded-md border border-border-subtle bg-bg-input p-[8px] font-mono text-[11px] leading-[1.5] text-text-secondary"
                  >
                    {login.lines.length === 0 ? strings.connect.outputEmpty : login.lines.slice(-12).join('\n')}
                  </pre>
                </div>
              </>
            ) : null}
          </>
        ) : null}

        {mode === 'api' ? (
          <>
            <div>
              <input
                type="password"
                id="connectKey"
                aria-label={strings.hub.keyLabel}
                placeholder={strings.hub.keyPlaceholder}
                value={key}
                onChange={(event) => setKey(event.target.value)}
                className="w-full rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted"
              />
              <div className="mt-[8px] flex flex-wrap items-center gap-[8px]">
                <button
                  type="button"
                  className={BTN_PRIMARY}
                  disabled={busy || key.trim() === ''}
                  id="connectSave"
                  onClick={() => {
                    void connectApiKey(providerId ?? '', key).then((saved) => {
                      if (saved) {
                        load(true);
                      }
                    });
                  }}
                >
                  {strings.hub.save}
                </button>
                <button type="button" className={BTN_SECONDARY} onClick={() => load(true)} disabled={busy} id="connectRefresh">
                  <RefreshCw size={13} /> {strings.connect.refreshModels}
                </button>
                {models !== null ? (
                  <span className="text-[11px] text-text-muted">{strings.connect.modelsSnapshot(models.snapshot)}</span>
                ) : null}
              </div>
            </div>

            <div>
              <div className="mb-[6px] flex items-center justify-between">
                <span className="text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted">
                  {strings.connect.modelsTitle}
                </span>
                <button type="button" className={BTN_SECONDARY} onClick={() => load(false)} disabled={busy} id="connectLoad">
                  {strings.connect.loadModels}
                </button>
              </div>

              {models === null || models.models.length === 0 ? (
                <p className="text-[12px] text-text-secondary" id="connectEmpty">
                  {strings.connect.modelsEmpty}
                </p>
              ) : (
                <ul className="max-h-[240px] overflow-y-auto" id="connectModels">
                  {models.models.map((model) => {
                    const inUse = models.selected.modelId === model.id;

                    return (
                      <li
                        key={`${model.providerId}/${model.id}`}
                        data-model={model.id}
                        data-source={model.source}
                        className="flex items-center gap-[10px] border-b border-border-subtle py-[7px] last:border-0"
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate font-mono text-[12px] text-text-primary">{model.id}</span>
                          <span className="block text-[11px] text-text-muted">
                            {model.tier} · {strings.connect.context(model.ctx)} · {model.cost || '—'}
                          </span>
                        </span>
                        {/* Where the row came from: "always up to date" is a claim worth showing. */}
                        <span
                          title={strings.connect.sourceHelp[model.source]}
                          className="shrink-0 rounded-full border border-border-subtle px-[8px] py-[2px] text-[10.5px] text-text-secondary"
                        >
                          {strings.connect.source[model.source]}
                        </span>
                        <button
                          type="button"
                          className={inUse ? BTN_SECONDARY : BTN_PRIMARY}
                          disabled={inUse}
                          onClick={() => use(model.id)}
                        >
                          {inUse ? strings.connect.selected : strings.connect.use}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}

              {models !== null && models.notes.length > 0 ? (
                <p className="mt-[8px] text-[11.5px] text-state-waiting" id="connectNotes">
                  {models.notes.join(' · ')}
                </p>
              ) : null}
            </div>
          </>
        ) : null}

        <div className="flex items-center justify-end gap-[8px]">
          <button
            type="button"
            className={BTN}
            id="connectClose"
            onClick={() => {
              if (login !== null && !login.authenticated) {
                void cancelCliLogin(login.loginId);
              }

              setLogin(null);
              close();
            }}
          >
            {strings.connect.cancel}
          </button>
        </div>

      </div>
    </Modal>
  );
}
