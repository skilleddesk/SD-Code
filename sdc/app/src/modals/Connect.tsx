import { useEffect, useRef, useState } from 'react';
import {
  Check,
  ClipboardCopy,
  ExternalLink,
  KeyRound,
  RefreshCw,
  Search,
  Terminal,
  TriangleAlert,
  Zap,
} from 'lucide-react';

import { strings } from '../strings';
import {
  cancelCliLogin,
  chooseModel,
  connectApiKey,
  loadCliRecipe,
  loadModels,
  pollCliLogin,
  startCliLogin,
  submitCliLoginCode,
  type CliLoginView,
  type CliRecipeView,
  type ModelsView,
} from '../store/intents';
import { useOverlayStore } from '../store/overlays';
import { useProviderStore } from '../store/providers';
import { tierFromName, tierLabel } from '../store/model';
import { toast } from '../store/toast';
import { Badge, type BadgeTone } from '../panels/ui/Badge';
import { BTN, BTN_GHOST, BTN_LG, BTN_PRIMARY, BTN_SECONDARY, BTN_SM, BTN_SM_LG } from '../panels/ui/button';
import { Field } from '../panels/ui/Field';
import { Section } from '../panels/ui/Section';
import { Modal } from './Modal';

/**
 * `#connectBd` - the two flows that get a user working (spec section 9.10), re-drawn in 0.7.1.
 *
 * The report that caused the redraw was a screenshot of the API-key dialog with everything on it circled:
 * *"koto useless and normal… button gulaw useless… sob gulatai aki"*. Read honestly, that dialog was one
 * flat column - an unlabelled password box, `Save` and `Refresh` squeezed together with a footnote
 * wrapping between them, a tiny `MODELS` heading, and rows carrying a hand-rolled pill, a lowercase tier
 * and a `Use` button shaped exactly like every other button on screen.
 *
 * The shape now is three parts, and every surface in this file follows it:
 *
 *   header    what this is: a provider tile, the name, a status badge, one sentence
 *   sections  the jobs: an uppercase name, the section's own controls, and a note under them
 *   footer    the dialog's one decision on the right, the way out beside it
 *
 * Nothing is decided twice: `Save key` is in the footer rather than beside `Refresh`, `Refresh` belongs
 * to the MODELS header, and the way out says `Close` - the same word as the frame's X.
 */
export function Connect() {
  const open = useOverlayStore((state) => state.connectOpen);
  const providerId = useOverlayStore((state) => state.connectProviderId);
  const mode = useOverlayStore((state) => state.connectMode);
  const close = useOverlayStore((state) => state.closeConnect);
  const providers = useProviderStore((state) => state.providers);

  const provider = providers.find((candidate) => candidate.id === providerId);
  const connected = provider?.status === 'connected';

  const [key, setKey] = useState('');
  const [login, setLogin] = useState<CliLoginView | null>(null);
  const [recipe, setRecipe] = useState<CliRecipeView | null>(null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [models, setModels] = useState<ModelsView | null>(null);
  const [filter, setFilter] = useState('');

  const startedFor = useRef<string | null>(null);
  const announced = useRef<string | null>(null);
  const codeRef = useRef<HTMLInputElement | null>(null);
  const outputRef = useRef<HTMLPreElement | null>(null);
  const focusedFor = useRef<string | null>(null);

  /* A finished sign-in says so out loud, once: the card flips because the daemon pushes a
     `ProviderStatus`, and this is the sentence beside it. */
  useEffect(() => {
    if (login === null || !login.authenticated || announced.current === login.loginId) {
      return;
    }

    announced.current = login.loginId;
    toast(strings.connect.signedInToast(login.providerLabel));
  }, [login]);

  /* The recipe, read before anything is started: it is what says whether the CLI this provider signs in
     through exists on this machine. Read on every open, because a person who just installed `claude` in a
     terminal and came back should see it. */
  useEffect(() => {
    if (!open || mode !== 'login' || providerId === undefined || providerId === null) {
      setRecipe(null);

      return;
    }

    void loadCliRecipe(providerId).then(setRecipe);
  }, [open, mode, providerId]);

  /* The sign-in starts itself: clicking a subscription card means "use my plan", and a person who just
     clicked it should not have to find a second button that says the same thing.
     Except when the CLI is not installed: then the recipe row is what they need, and starting a login to
     report "not found" would be the failure explaining itself instead of a step to take first (0.7.8). */
  useEffect(() => {
    if (!open) {
      startedFor.current = null;
      return;
    }

    if (mode !== 'login' || providerId === null || providerId === undefined || startedFor.current === providerId) {
      return;
    }

    /* `null` means "not read yet" for a provider that has no recipe at all - so this waits for the read
       rather than guessing, and a provider without a recipe (an API key one) never auto-starts a login. */
    if (recipe === null || !recipe.installed) {
      return;
    }

    startedFor.current = providerId;
    setBusy(true);

    void startCliLogin(providerId).then((view) => {
      setBusy(false);
      setLogin(view);
    });
  }, [open, mode, providerId, recipe]);

  /* One poll per second while a sign-in is in flight, and none after it has spoken. */
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

  /* The paste boxes are drawn only while the CLI is still waiting for a code. A sign-in that has stopped
     keeps no field: a box under a CLI that already said `Login failed` invites a person to paste into
     something that cannot answer. */
  const waitingForCode =
    login !== null &&
    !login.authenticated &&
    login.url !== null &&
    (login.state === 'waiting_for_code' || login.state === 'waiting_for_url');

  /* The code field takes focus once, keyed by login id: the poll re-renders this dialog every second, and
     a field that is re-focused on every render cannot be typed into. */
  useEffect(() => {
    if (login === null || !waitingForCode || focusedFor.current === login.loginId) {
      return;
    }

    focusedFor.current = login.loginId;
    codeRef.current?.focus();
  }, [login, waitingForCode]);

  /* The CLI's own output scrolls inside its own fixed-height box, so one line more or less cannot resize
     the dialog and push the fields out from under the pointer. */
  useEffect(() => {
    const output = outputRef.current;

    if (output !== null) {
      output.scrollTop = output.scrollHeight;
    }
  }, [login]);

  /* A sign-in that finished closes itself, after long enough to read the line that says so. */
  useEffect(() => {
    if (login === null || !login.authenticated) {
      return undefined;
    }

    const timer = window.setTimeout(close, 2600);

    return () => window.clearTimeout(timer);
  }, [login, close]);

  const shown =
    models === null
      ? []
      : models.models.filter((model) =>
          filter.trim() === ''
            ? true
            : `${model.id} ${model.name}`.toLowerCase().includes(filter.trim().toLowerCase()),
        );

  const copy = (text: string): void => {
    void navigator.clipboard?.writeText(text).then(() => toast(strings.connect.copied));
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

  const saveKey = (): void => {
    setBusy(true);
    void connectApiKey(providerId ?? '', key).then((saved) => {
      setBusy(false);

      if (saved) {
        setKey('');
        load(true);
      }
    });
  };

  const use = (modelId: string): void => {
    void chooseModel(modelId, providerId ?? '').then((ok) => {
      if (ok) {
        setModels((current) =>
          current === null ? current : { ...current, selected: { modelId, providerId: providerId ?? null } },
        );
        toast(`${modelId} · ${strings.connect.selected}`);
      }
    });
  };

  const leave = (): void => {
    if (login !== null && !login.authenticated && login.state !== 'exited') {
      void cancelCliLogin(login.loginId);
    }

    setLogin(null);
    close();
  };


  return (
    <Modal open={open} label={`${strings.connect.title} · ${name}`} onClose={leave} center className="connect-dlg w-[min(620px,96vw)]">
      <div className="flex flex-col" data-connect={mode} data-connect-provider={providerId ?? ''}>
        {/* ---------------------------------------------------------------- header */}
        <header className="flex items-start gap-[12px] border-b border-border-subtle px-[18px] py-[16px]">
          <div className="grid h-[36px] w-[36px] shrink-0 place-items-center rounded-lg bg-accent-subtle text-accent">
            {mode === 'login' ? <Terminal size={18} aria-hidden="true" /> : <KeyRound size={18} aria-hidden="true" />}
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-[8px]">
              <h2 className="truncate text-[14px] font-semibold text-text-primary">
                {mode === 'login' ? strings.connect.loginTitle : strings.connect.apiTitle}
              </h2>

              <Badge tone={connected ? 'success' : 'neutral'}>
                {connected ? strings.connect.verifiedBadge : strings.connect.notConnectedBadge}
              </Badge>
            </div>

            <p className="mt-[3px] text-[12px] leading-[1.55] text-text-secondary">
              {mode === 'login' ? strings.connect.loginBody : `${name} · ${strings.connect.apiBody}`}
            </p>
          </div>
        </header>

        {/* ------------------------------------------------------------------ body */}
        <div className="flex flex-col">
          {mode === 'login' ? (
            <>
              {/*
                The recipe, before the sign-in: `claude` / `codex` / `gemini` are separate programs, and this
                row is where "install it first" is said - as a step, not as the aftermath of a failure.
                It is drawn whether the program is there or not: `claude is installed` is information too,
                and a row that only appears on failure would be a row nobody can find when it matters.
              */}
              {recipe === null ? null : (
                <div
                  className="connect-recipe flex items-start gap-[8px] border-b border-border-subtle px-[18px] py-[12px] text-[11.5px] leading-[1.55]"
                  id="connectRecipe"
                  data-recipe-program={recipe.program}
                  data-recipe-installed={recipe.installed ? 'true' : 'false'}
                >
                  {recipe.installed ? (
                    <Check size={13} className="mt-[2px] shrink-0 text-state-success" aria-hidden="true" />
                  ) : (
                    <TriangleAlert size={13} className="mt-[2px] shrink-0 text-state-waiting" aria-hidden="true" />
                  )}

                  <div className="min-w-0 flex-1">
                    <div className={recipe.installed ? 'text-text-secondary' : 'font-semibold text-text-primary'}>
                      {recipe.installed
                        ? strings.connect.recipe.present(recipe.program)
                        : strings.connect.recipe.missing(recipe.program)}
                    </div>

                    {recipe.installed ? null : (
                      <>
                        <div className="mt-[2px] text-text-secondary">{strings.connect.recipe.missingBody}</div>
                        <pre className="mt-[6px] overflow-x-auto rounded-sm border border-border-subtle bg-bg-input px-[8px] py-[6px] font-mono text-[11px] text-text-primary">
                          {recipe.note}
                        </pre>

                        <div className="mt-[8px] flex items-center gap-[6px]">
                          <button
                            type="button"
                            id="connectRecipeCopy"
                            className={BTN_SM + ' ' + BTN_SECONDARY}
                            onClick={() => copy(recipe.note)}
                          >
                            <ClipboardCopy size={11} aria-hidden="true" /> {strings.connect.recipe.copy}
                          </button>
                          <button
                            type="button"
                            id="connectRecipeRecheck"
                            className={BTN_SM + ' ' + BTN_SECONDARY}
                            onClick={() => {
                              void loadCliRecipe(recipe.providerId).then((next) => {
                                setRecipe(next);
                                toast(strings.connect.recipe.recheckToast);
                              });
                            }}
                          >
                            <RefreshCw size={11} aria-hidden="true" /> {strings.connect.recipe.recheck}
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                </div>
              )}

              {login === null ? (
                <Section title={strings.connect.pageTitle} note={strings.connect.note}>
                  <div className="flex items-center gap-[10px]">
                    <button
                      type="button"
                      className={BTN_LG + ' ' + BTN_PRIMARY}
                      onClick={signIn}
                      disabled={busy}
                      id="connectSignIn"
                    >
                      <Terminal size={14} aria-hidden="true" /> {strings.connect.signIn}
                    </button>
                    <span className="text-[11.5px] text-text-muted">{name}</span>
                  </div>
                </Section>
              ) : null}

              {login !== null ? (
                <>
                  <div className="border-b border-border-subtle px-[18px] py-[14px]">
                    <div className="flex items-start gap-[8px] text-[12px] leading-[1.55]">
                      {login.authenticated ? (
                        <>
                          <Check size={14} className="mt-[1px] shrink-0 text-state-success" aria-hidden="true" />
                          <span className="text-text-secondary">
                            <span className="font-semibold text-state-success">{strings.connect.authenticated}</span>
                            {' · '}
                            {strings.connect.signedInNote}
                          </span>
                        </>
                      ) : login.state === 'failed' || login.state === 'cancelled' ? (
                        <>
                          <TriangleAlert size={14} className="mt-[1px] shrink-0 text-state-error" aria-hidden="true" />
                          <span className="text-text-secondary">{strings.connect.failed}</span>
                        </>
                      ) : login.state === 'exited' ? (
                        <>
                          <TriangleAlert size={14} className="mt-[1px] shrink-0 text-state-waiting" aria-hidden="true" />
                          <span className="text-text-secondary">{strings.connect.finished}</span>
                        </>
                      ) : (
                        <>
                          <RefreshCw
                            size={14}
                            className="mt-[1px] shrink-0 animate-spin text-state-waiting"
                            aria-hidden="true"
                          />
                          <span className="text-text-secondary">
                            {login.state === 'waiting_for_code'
                              ? strings.connect.waitingForCode
                              : login.state === 'waiting_for_browser'
                                ? strings.connect.waitingForBrowser
                                : strings.connect.waiting}
                          </span>
                        </>
                      )}
                    </div>

                    {waitingForCode ? (
                      <div className="mt-[12px] flex flex-col gap-[8px]">
                        <Field
                          inputRef={codeRef}
                          id="connectCode"
                          label={strings.connect.codeLabel}
                          placeholder={strings.connect.codePlaceholder}
                          value={code}
                          onChange={setCode}
                          onEnter={() => {
                            if (code.trim() !== '') {
                              submit();
                            }
                          }}
                          hint={strings.connect.codeHint}
                        />

                        <div className="flex justify-end">
                          <button
                            type="button"
                            className={BTN + ' ' + BTN_PRIMARY}
                            onClick={submit}
                            disabled={busy || code.trim() === ''}
                            id="connectSubmit"
                          >
                            {strings.connect.submitCode}
                          </button>
                        </div>
                      </div>
                    ) : null}

                    {!login.authenticated &&
                    (login.state === 'failed' || login.state === 'cancelled' || login.state === 'exited') ? (
                      <div className="mt-[10px]">
                        <button
                          type="button"
                          className={BTN + ' ' + BTN_SECONDARY}
                          onClick={signIn}
                          disabled={busy}
                          id="connectRetry"
                        >
                          <RefreshCw size={13} aria-hidden="true" /> {strings.connect.tryAgain}
                        </button>
                      </div>
                    ) : null}
                  </div>
                  {login.url !== null && waitingForCode ? (
                    <Section
                      title={strings.connect.pageLabel}
                      hint={login.providerLabel}
                      action={
                        <>
                          <button
                            type="button"
                            className={BTN_SM + ' ' + BTN_GHOST}
                            onClick={() => copy(login.url ?? '')}
                            id="connectCopy"
                          >
                            <ClipboardCopy size={12} aria-hidden="true" /> {strings.connect.copyLink}
                          </button>
                          <a
                            className={BTN_SM + ' ' + BTN_GHOST}
                            href={login.url ?? '#'}
                            target="_blank"
                            rel="noreferrer noopener"
                          >
                            <ExternalLink size={12} aria-hidden="true" /> {strings.connect.openLink}
                          </a>
                        </>
                      }
                    >
                      <input
                        readOnly
                        id="connectUrl"
                        aria-label={strings.connect.copyLink}
                        value={login.url ?? ''}
                        className="w-full rounded-md border border-border-subtle bg-bg-input px-[10px] py-[7px] font-mono text-[11.5px] text-text-secondary"
                      />
                    </Section>
                  ) : null}

                  <Section
                    title={strings.connect.outputTitle}
                    hint={strings.connect.outputHint}
                    action={
                      <button
                        type="button"
                        className={BTN_SM + ' ' + BTN_GHOST}
                        onClick={() => copy(login.lines.join('\n'))}
                        disabled={login.lines.length === 0}
                      >
                        <ClipboardCopy size={12} aria-hidden="true" /> {strings.connect.copyOutput}
                      </button>
                    }
                    note={login.note === null ? undefined : login.note}
                  >
                    <pre
                      ref={outputRef}
                      id="connectOutput"
                      className="h-[132px] overflow-auto rounded-md border border-border-subtle bg-bg-input p-[10px] font-mono text-[11px] leading-[1.55] text-text-secondary"
                    >
                      {login.lines.length === 0 ? strings.connect.outputEmpty : login.lines.slice(-12).join('\n')}
                    </pre>
                  </Section>
                </>
              ) : null}
            </>
          ) : null}

          {mode === 'api' ? (
            <>
              <Section
                title={strings.connect.keyTitle}
                action={
                  <Badge tone={key.trim() === '' ? 'muted' : 'warning'}>
                    {key.trim() === ''
                      ? connected
                        ? strings.connect.keySaved
                        : strings.connect.keyNotSaved
                      : strings.connect.keyEditing}
                  </Badge>
                }
                note={strings.connect.keyNote}
              >
                <Field
                  id="connectKey"
                  label={strings.connect.keyLabel}
                  placeholder={strings.connect.keyPlaceholder}
                  value={key}
                  onChange={setKey}
                  secret
                  hint={strings.connect.keyHint(name)}
                  onEnter={() => {
                    if (key.trim() !== '') {
                      saveKey();
                    }
                  }}
                />
              </Section>

              <Section
                title={strings.connect.modelsTitle}
                hint={
                  models === null
                    ? undefined
                    : strings.connect.modelsHint(shown.length, models.models.length, models.snapshot)
                }
                action={
                  <>
                    <button
                      type="button"
                      className={BTN_SM + ' ' + BTN_GHOST}
                      onClick={() => load(false)}
                      disabled={busy}
                      id="connectLoad"
                    >
                      {strings.connect.loadModels}
                    </button>
                    <button
                      type="button"
                      className={BTN_SM + ' ' + BTN_GHOST}
                      onClick={() => load(true)}
                      disabled={busy}
                      id="connectRefresh"
                    >
                      <RefreshCw size={12} aria-hidden="true" /> {strings.connect.refresh}
                    </button>
                  </>
                }
                note={
                  models !== null && models.notes.length > 0 ? (
                    <span id="connectNotes" className="text-state-waiting">
                      {models.notes.join(' · ')}
                    </span>
                  ) : undefined
                }
              >
                {models === null || models.models.length === 0 ? (
                  <p className="text-[12px] text-text-secondary" id="connectEmpty">
                    {strings.connect.modelsEmpty}
                  </p>
                ) : (
                  <>
                    {models.models.length > 5 ? (
                      <div className="relative mb-[8px]">
                        <Search
                          size={12}
                          aria-hidden="true"
                          className="pointer-events-none absolute left-[9px] top-[9px] text-text-muted"
                        />
                        <input
                          id="connectFilter"
                          aria-label={strings.connect.modelFilter}
                          placeholder={strings.connect.modelFilter}
                          value={filter}
                          onChange={(event) => setFilter(event.target.value)}
                          className="w-full rounded-md border border-border-subtle bg-bg-input py-[7px] pl-[27px] pr-[10px] text-[12px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
                        />
                      </div>
                    ) : null}

                    {shown.length === 0 ? (
                      <p className="text-[12px] text-text-muted" id="connectNoMatch">
                        {strings.connect.modelNoMatch}
                      </p>
                    ) : (
                      <ul className="flex max-h-[240px] flex-col gap-[4px] overflow-y-auto" id="connectModels">
                        {shown.map((model) => {
                          const inUse = models.selected.modelId === model.id;
                          const tier = tierLabel(tierFromName(String(model.tier)));

                          return (
                            <li
                              key={`${model.providerId}/${model.id}`}
                              data-model={model.id}
                              data-source={model.source}
                              className={
                                'model-row flex items-center gap-[10px] rounded-md border px-[10px] py-[8px] transition-colors duration-fast ease-ease ' +
                                (inUse
                                  ? 'border-accent bg-accent-subtle'
                                  : 'border-border-subtle bg-bg-raised hover:border-border-default')
                              }
                            >
                              <div className="min-w-0 flex-1">
                                <div className="truncate text-[12.5px] font-medium text-text-primary">
                                  {model.name === '' ? model.id : model.name}
                                </div>
                                <div className="mt-[1px] truncate font-mono text-[10.5px] text-text-muted">
                                  {model.id}
                                </div>
                              </div>

                              <div className="flex shrink-0 items-center gap-[5px]">
                                <Badge tone="muted">{tier}</Badge>
                                <Badge tone="muted">{strings.connect.context(model.ctx)}</Badge>
                                <Badge tone="muted">{model.cost === '' ? '—' : model.cost}</Badge>
                                <Badge
                                  tone={SOURCE_TONE[model.source] ?? 'neutral'}
                                  icon={Zap}
                                  title={strings.connect.sourceHelp[model.source]}
                                >
                                  {strings.connect.source[model.source]}
                                </Badge>
                              </div>

                              {inUse ? (
                                <span className="flex h-[24px] shrink-0 items-center gap-[5px] rounded-md bg-green-subtle px-[9px] text-[10.5px] font-semibold text-state-success">
                                  <Check size={11} aria-hidden="true" /> {strings.connect.selected}
                                </span>
                              ) : (
                                <button
                                  type="button"
                                  className={BTN_SM + ' ' + BTN_SECONDARY}
                                  onClick={() => use(model.id)}
                                >
                                  {strings.connect.use}
                                </button>
                              )}
                            </li>
                          );
                        })}
                      </ul>
                    )}
                  </>
                )}
              </Section>
            </>
          ) : null}
        </div>

        {/* ---------------------------------------------------------------- footer */}
        <footer className="flex items-center gap-[10px] border-t border-border-subtle bg-bg-raised px-[18px] py-[12px]">
          <span className="min-w-0 truncate text-[11px] text-text-muted">
            {mode === 'login' ? strings.connect.footer.login : strings.connect.footer.api}
          </span>

          <div className="ml-auto flex shrink-0 items-center gap-[8px]">
            <button type="button" className={BTN_SM_LG + ' ' + BTN_SECONDARY} id="connectClose" onClick={leave}>
              {strings.connect.close}
            </button>

            {mode === 'api' ? (
              <button
                type="button"
                className={BTN_SM_LG + ' ' + BTN_PRIMARY}
                id="connectSave"
                disabled={busy || key.trim() === ''}
                onClick={saveKey}
              >
                <KeyRound size={13} aria-hidden="true" /> {strings.connect.saveKey}
              </button>
            ) : null}
          </div>
        </footer>
      </div>
    </Modal>
  );
}

/** The tone a row's `source` badge gets: the provider's own answer is the good one. */
const SOURCE_TONE: Record<string, BadgeTone> = {
  live: 'success',
  cache: 'neutral',
  bundled: 'muted',
};

