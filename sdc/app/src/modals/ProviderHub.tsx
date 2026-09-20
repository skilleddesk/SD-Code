import { useState, type ReactNode } from 'react';
import {
  CircleCheckBig,
  CirclePlus,
  Crown,
  Grid3x3,
  HardDrive,
  Key,
  List,
  Plug,
  ShieldCheck,
  Stethoscope,
  TriangleAlert,
  X,
  Zap,
  type LucideIcon,
} from 'lucide-react';

import { strings } from '../strings';
import {
  authorizeSubscription,
  connectApiKey,
  connectLocal,
  runDoctor,
  saveCustomEndpoint,
  testProvider,
  toggleRegistryModel,
} from '../store/intents';
import { useProviderStore } from '../store/providers';
import { useAppStore } from '../store/store';
import type { DoctorCheckView, ProviderView } from '../store/types';
import { useOverlayStore, type HubTab } from '../store/overlays';
import { toast } from '../store/toast';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * `#providerBd` - the Provider Hub (spec section 9.10).
 *
 * 920x680, a 220px nav and the body. Seven nav items; six flows behind them:
 *
 *   1  API key        password field + label, `Test connection` (1.2s of spinner, then
 *                     `OK · key valid · 12 models available`) and `Save` (keychain).
 *   2  Subscription   the OAuth note, `Open browser login`, then `Authorized · token received`.
 *   3  Local          the Ollama doctor rows and `Connect`.
 *   4  Custom         URL + key + protocol, then `Save`.
 *   5  Registry       the twelve models with a toggle each and a tier badge.
 *   6  Doctor         the ten environment checks, each warn/fail carrying its `Fix`.
 *
 * Every one of those flows is a *call*, not a local animation: the spinner runs while
 * `provider.test` is in flight, the success line is the daemon's structured answer, and a save ends
 * with a `ProviderStatus` event that the card, the topbar dot and the status bar all pick up. The
 * hub therefore keeps no state of its own beyond "which field is being typed into".
 */
const NAV_ICON: Record<string, LucideIcon> = {
  grid: Grid3x3,
  crown: Crown,
  key: Key,
  hardDrive: HardDrive,
  plug: Plug,
  list: List,
  stethoscope: Stethoscope,
};

/** The three inline flows: a key, an OAuth round trip, or the local daemon. */
interface FlowState {
  providerId: string;
  kind: 'api-key' | 'oauth' | 'local';
}

export function ProviderHub() {
  const open = useOverlayStore((state) => state.hubOpen);
  const close = useOverlayStore((state) => state.closeHub);
  const tab = useOverlayStore((state) => state.hubTab);
  const setTab = useOverlayStore((state) => state.openHub);
  const providers = useProviderStore((state) => state.providers);
  const hosts = useAppStore((state) => state.hosts);
  const doctor = useAppStore((state) => state.doctor);

  const [flow, setFlow] = useState<FlowState | null>(null);

  const titles = strings.hub.titles[tab];

  return (
    <Modal
      open={open}
      label={strings.hub.navTitle}
      onClose={close}
      center
      className="hub-dlg flex h-[min(680px,92vh)] w-[min(920px,96vw)] overflow-hidden"
    >
      <nav className="flex w-[220px] shrink-0 flex-col gap-[2px] overflow-y-auto border-r border-border-subtle bg-bg-raised p-[10px] max-700:w-[64px]">
        <div className="px-[12px] pb-[8px] pt-[6px] text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted max-700:hidden">
          {strings.hub.navTitle}
        </div>

        {strings.hub.nav.map((item, index) => {
          const Icon = NAV_ICON[item.icon] ?? Grid3x3;
          const selected = item.id === tab;

          return (
            <div key={item.id}>
              {index === 5 ? (
                <div className="px-[12px] pb-[6px] pt-[14px] text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted max-700:hidden">
                  {strings.hub.navConfigTitle}
                </div>
              ) : null}

              <button
                type="button"
                data-hub={item.id}
                aria-current={selected}
                title={item.label}
                className={
                  'flex w-full items-center gap-[10px] rounded-md px-[12px] py-[8px] text-left text-[12.5px] transition-colors duration-fast ease-ease ' +
                  (selected
                    ? 'active bg-accent-subtle text-accent'
                    : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
                }
                onClick={() => {
                  setFlow(null);
                  setTab(item.id as HubTab);
                }}
              >
                <Icon size={14} aria-hidden="true" />
                <span className="max-700:hidden">{item.label}</span>
              </button>
            </div>
          );
        })}
      </nav>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-start gap-[10px] border-b border-border-subtle px-[18px] py-[14px]">
          <div className="flex-1">
            <div className="text-[14px] font-semibold text-text-primary">{titles[0]}</div>
            <div className="mt-[2px] text-[11.5px] text-text-muted">{titles[1]}</div>
          </div>
          <button
            type="button"
            className="grid h-[28px] w-[28px] place-items-center rounded-md text-text-secondary hover:bg-bg-hover hover:text-text-primary"
            title={strings.hub.close}
            aria-label={strings.hub.close}
            onClick={close}
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>

        <div
          className="min-h-0 flex-1 overflow-y-auto p-[16px]"
          data-hub-body={flow === null ? tab : flow.providerId}
        >
          {flow !== null ? (
            <Flow
              flow={flow}
              onDone={() => {
                setFlow(null);
                setTab('all');
              }}
            />
          ) : tab === 'registry' ? (
            <RegistryList />
          ) : tab === 'doctor' ? (
            <DoctorList
              checks={doctor['local'] ?? []}
              onRun={() => void runDoctor('local')}
              ready={hosts.length > 0}
            />
          ) : tab === 'custom' ? (
            <CustomEndpoint onSaved={() => setTab('all')} />
          ) : (
            <ProviderCards
              providers={providers.filter((provider) =>
                tab === 'all'
                  ? true
                  : tab === 'subscriptions'
                    ? provider.kind === 'subscription'
                    : tab === 'api-keys'
                      ? provider.kind === 'api-key'
                      : provider.kind === 'local',
              )}
              onConnect={(providerId, kind) => setFlow({ providerId, kind })}
            />
          )}
        </div>
      </div>
    </Modal>
  );
}

/* ------------------------------------------------------------------------------------------------
 * The provider cards (the `all`, `subscriptions`, `api-keys` and `local` tabs)
 * ---------------------------------------------------------------------------------------------- */

interface ProviderCardsProps {
  providers: ProviderView[];
  onConnect: (providerId: string, kind: FlowState['kind']) => void;
}

function ProviderCards({ providers, onConnect }: ProviderCardsProps) {
  const connected = providers.filter((provider) => provider.status === 'connected');
  const rest = providers.filter((provider) => provider.status !== 'connected');

  const card = (provider: ProviderView) => (
    <button
      key={provider.id}
      type="button"
      data-provider={provider.id}
      data-status={provider.status}
      className={
        'prov-card flex flex-col gap-[8px] rounded-lg border bg-bg-raised p-[12px] text-left transition-all duration-fast ease-ease hover:border-border-strong ' +
        (provider.status === 'connected' ? 'border-border-subtle' : 'border-border-default')
      }
      onClick={() => {
        /* A subscription signs in through its CLI, an API provider needs a key and a model: both are
           the Connect modal's job (spec section 9.10). A local provider is the two doctor rows, which
           stay in the card's own inline flow. */
        if (provider.kind === 'local') {
          onConnect(provider.id, 'local');
          return;
        }

        useOverlayStore.getState().openConnect(provider.id, provider.kind === 'subscription' ? 'login' : 'api');
      }}
    >
      <span className="flex items-center gap-[10px]">
        <span
          className={
            'prov-logo grid h-[34px] w-[34px] shrink-0 place-items-center rounded-full text-[13px] font-bold text-text-on-accent ' +
            provider.logo
          }
          aria-hidden="true"
        >
          {provider.initial}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-semibold text-text-primary">
            {provider.name}
          </span>
          <span className="block text-[10.5px] text-text-muted">
            {strings.hub.kind[provider.kind]}
          </span>
        </span>
      </span>

      <span className="text-[11.5px] leading-[1.5] text-text-secondary">{provider.detail}</span>

      <span className="flex items-center gap-[8px]">
        <span
          className={
            'rounded-full px-[7px] py-[1px] text-[10px] font-semibold ' +
            (provider.status === 'connected'
              ? 'bg-green-subtle text-state-success'
              : provider.status === 'needs-auth'
                ? 'bg-orange-subtle text-state-warning'
                : 'bg-bg-base text-text-muted')
          }
        >
          {strings.hub.status[provider.status]}
        </span>
        <span className="flex-1" />
        <span
          className={
            'inline-flex h-[24px] items-center rounded-md border px-[9px] text-[10.5px] font-medium ' +
            (provider.status === 'connected'
              ? 'border-border-default bg-bg-raised text-text-primary'
              : 'border-accent bg-accent text-text-on-accent')
          }
        >
          {provider.status === 'connected' ? strings.hub.manage : strings.hub.connect}
        </span>
      </span>
    </button>
  );

  return (
    <div className="flex flex-col gap-[16px]">
      <section>
        <h3 className="mb-[8px] flex items-center gap-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
          <CircleCheckBig size={12} aria-hidden="true" className="text-state-success" />
          {strings.hub.connectedCount(connected.length)}
        </h3>

        {connected.length === 0 ? (
          <div className="p-[12px] text-[12px] text-text-muted">{strings.hub.noneYet}</div>
        ) : (
          <div className="grid grid-cols-2 gap-[10px] max-700:grid-cols-1">{connected.map(card)}</div>
        )}
      </section>

      {rest.length > 0 ? (
        <section>
          <h3 className="mb-[8px] flex items-center gap-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
            <CirclePlus size={12} aria-hidden="true" />
            {strings.hub.availableTitle}
          </h3>
          <div className="grid grid-cols-2 gap-[10px] max-700:grid-cols-1">{rest.map(card)}</div>
        </section>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------------------------------------------
 * The inline flows a card's Connect button opens (spec section 9.10, flows 1-3)
 * ---------------------------------------------------------------------------------------------- */

/**
 * A stable empty list, so `state.doctor['local'] ?? EMPTY_CHECKS` hands zustand the same reference on
 * every render. A fresh `[]` would be a new array each time and re-render forever - the same mistake
 * that took the window down in 0.4.4 (`overlays/Toast.tsx`), which is why it is a named constant.
 */
const EMPTY_CHECKS: readonly DoctorCheckView[] = [];

function Flow({ flow, onDone }: { flow: FlowState; onDone: () => void }) {
  const providers = useProviderStore((state) => state.providers);
  const provider = providers.find((candidate) => candidate.id === flow.providerId);
  const [key, setKey] = useState('');
  const [label, setLabel] = useState('');
  const [state, setState] = useState<'idle' | 'busy' | 'ok' | 'fail'>('idle');
  const [detail, setDetail] = useState('');

  /* The `host.doctor` rows the daemon returned for this machine. Empty until a run: the card then
     says so rather than printing an optimistic one of its own. */
  const localChecks = useAppStore((state) => state.doctor['local'] ?? EMPTY_CHECKS);

  const name = provider?.name ?? flow.providerId;

  /** Flow 1's `Test connection`: 1.2s of spinner, then the daemon's structured answer. */
  const test = (): void => {
    setState('busy');
    setDetail(strings.hub.testing);

    void testProvider(flow.providerId, key).then((result) => {
      if (result === null || !result.ok) {
        setState('fail');
        setDetail(result?.error ?? strings.hub.testEmpty);
        return;
      }

      setState('ok');
      setDetail(result.detail || strings.hub.testOk(result.models));
    });
  };

  const save = (): void => {
    setState('busy');

    void connectApiKey(flow.providerId, key, label).then((saved) => {
      if (saved) {
        onDone();
      } else {
        setState('fail');
      }
    });
  };

  /** Flow 2: the browser does the login; SDC only receives the token. */
  const authorize = (): void => {
    setState('busy');
    setDetail(strings.hub.oauthWaiting);

    void authorizeSubscription(flow.providerId).then((ok) => {
      setState(ok ? 'ok' : 'fail');
      setDetail(ok ? strings.hub.oauthOk : strings.hub.testFail(name));

      if (ok) {
        window.setTimeout(onDone, 600);
      }
    });
  };

  /** Flow 3: the two doctor rows of the local daemon. */
  const local = (): void => {
    setState('busy');

    void connectLocal().then((result) => {
      const up = result?.daemon === true;

      setState(up ? 'ok' : 'fail');

      if (up) {
        onDone();
      }
    });
  };

  return (
    <div className="max-w-[520px]" data-flow={flow.kind} data-flow-provider={flow.providerId}>
      {flow.kind === 'api-key' ? (
        <>
          <Field label={strings.hub.keyLabel} required>
            <input
              type="password"
              id="apiKeyInput"
              aria-label={strings.hub.keyLabel}
              className="w-full rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
              placeholder={strings.hub.keyPlaceholder}
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
            <p className="mt-[4px] text-[11px] text-text-muted">
              {strings.hub.keyHint(name)} · {strings.hub.keychainNote}
            </p>
          </Field>

          <Field label={strings.hub.labelField}>
            <input
              type="text"
              id="apiKeyLabel"
              className="w-full rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
              placeholder={strings.hub.labelPlaceholder}
              value={label}
              onChange={(event) => setLabel(event.target.value)}
            />
          </Field>

          <ConnTest state={state} detail={detail} />

          <div className="mt-[16px] flex gap-[8px]">
            <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={onDone}>
              {strings.hub.back}
            </button>
            <button type="button" className={BTN + ' ' + BTN_SECONDARY} data-action="test" onClick={test}>
              <Zap size={12} aria-hidden="true" />
              {strings.hub.test}
            </button>
            <button type="button" className={BTN + ' ' + BTN_PRIMARY} data-action="save" onClick={save}>
              {strings.hub.save}
            </button>
          </div>
        </>
      ) : null}

      {flow.kind === 'oauth' ? (
        <>
          <div className="mb-[16px] flex items-start gap-[8px] rounded-md bg-accent-subtle px-[12px] py-[9px] text-[11.5px] text-text-secondary">
            <ShieldCheck size={14} aria-hidden="true" className="mt-[1px] shrink-0 text-accent" />
            {strings.hub.subscriptionNote(name)}
          </div>

          <ConnTest state={state} detail={detail} />

          <div className="mt-[16px] flex gap-[8px]">
            <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={onDone}>
              {strings.hub.back}
            </button>
            <button
              type="button"
              className={BTN + ' ' + BTN_PRIMARY}
              data-action="oauth"
              disabled={state === 'busy'}
              onClick={authorize}
            >
              {strings.hub.openBrowser}
            </button>
          </div>
        </>
      ) : null}

      {flow.kind === 'local' ? (
        <>
          {/*
            The two rows that used to be printed here were fixed: `daemon running` and
            `Ollama installed · 3 models`, whether or not anything was installed. Now the tab shows
            the `host.doctor` rows the daemon actually returned for this machine, and - with no run
            yet - the sentence that says so and the button that runs it.
          */}
          {localChecks.length === 0 ? (
            <p className="mb-[12px] text-[12.5px] text-text-secondary" id="hubLocalEmpty">
              {strings.hub.localEmpty}
            </p>
          ) : (
            localChecks.map((check) => (
              <DoctorRow
                key={check.id}
                state={check.state}
                label={check.label}
                detail={check.detail}
                fix={check.fix}
              />
            ))
          )}

          <div className="mt-[16px] flex gap-[8px]">
            <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={onDone}>
              {strings.hub.back}
            </button>
            <button
              type="button"
              className={BTN + ' ' + BTN_PRIMARY}
              data-action="local-connect"
              onClick={local}
            >
              {strings.hub.connect}
            </button>
          </div>
        </>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------------------------------------------
 * The four remaining flows and the small pieces they share
 * ---------------------------------------------------------------------------------------------- */

/** Flow 4: any OpenAI- or Anthropic-compatible endpoint. */
function CustomEndpoint({ onSaved }: { onSaved: () => void }) {
  const [url, setUrl] = useState('');
  const [key, setKey] = useState('');
  const [protocol, setProtocol] = useState<string>(strings.hub.protocols[0]);

  return (
    <div className="max-w-[520px]" data-flow="custom">
      <Field label={strings.hub.customUrl} required>
        <input
          type="text"
          id="custUrl"
          className="w-full rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
          placeholder="https://api.example.com/v1"
          value={url}
          onChange={(event) => setUrl(event.target.value)}
        />
      </Field>

      <Field label={strings.hub.customKey}>
        <input
          type="password"
          id="custKey"
          className="w-full rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
          placeholder={strings.hub.keyPlaceholder}
          value={key}
          onChange={(event) => setKey(event.target.value)}
        />
      </Field>

      <Field label={strings.hub.customProtocol}>
        <select
          className="w-full rounded-md border border-border-default bg-bg-raised px-[10px] py-[7px] text-[12.5px] text-text-primary"
          value={protocol}
          onChange={(event) => setProtocol(event.target.value)}
        >
          {strings.hub.protocols.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      </Field>

      <button
        type="button"
        className={BTN + ' ' + BTN_PRIMARY}
        data-action="save-endpoint"
        onClick={() =>
          void saveCustomEndpoint({ url, key, protocol }).then((saved) => {
            if (saved) {
              onSaved();
            }
          })
        }
      >
        {strings.hub.save}
      </button>
    </div>
  );
}

/** Flow 5: the model registry. One row per model, a checkbox and a tier badge. */
function RegistryList() {
  const models = useAppStore((state) => state.registry);

  return (
    <section>
      <h3 className="mb-[8px] flex items-center gap-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
        <List size={12} aria-hidden="true" />
        {strings.hub.registryTitle(models.length)}
      </h3>

      {models.map((model) => (
        <button
          key={model.id}
          type="button"
          data-model={model.id}
          data-enabled={model.enabled}
          aria-pressed={model.enabled}
          className={
            'mb-[4px] flex w-full items-center gap-[10px] rounded-md border px-[12px] py-[9px] text-left transition-colors duration-fast ease-ease ' +
            (model.enabled
              ? 'border-accent bg-accent-subtle'
              : 'border-border-subtle bg-bg-raised hover:border-border-strong')
          }
          onClick={() => void toggleRegistryModel(model.id, !model.enabled)}
        >
          <span
            className={
              'grid h-[15px] w-[15px] shrink-0 place-items-center rounded-[4px] border ' +
              (model.enabled ? 'border-accent bg-accent' : 'border-border-default')
            }
            aria-hidden="true"
          >
            {model.enabled ? <CircleCheckBig size={11} className="text-text-on-accent" /> : null}
          </span>

          <span className="min-w-0 flex-1">
            <span className="block truncate font-mono text-[12px] text-text-primary">{model.id}</span>
            <span className="mt-[2px] block font-mono text-[10.5px] text-text-muted">
              {strings.hub.registryCtx(Math.round(model.ctx / 1000))} · {model.cost}
            </span>
          </span>

          <span
            className={
              'rounded-full px-[7px] py-[1px] text-[10px] font-semibold ' +
              (model.tier === 'deep'
                ? 'bg-purple-subtle text-purple'
                : model.tier === 'balanced'
                  ? 'bg-accent-subtle text-accent'
                  : 'bg-green-subtle text-state-success')
            }
          >
            {model.tier}
          </span>
        </button>
      ))}
    </section>
  );
}

/** Flow 6: the ten environment checks. Warn and fail rows carry their own `Fix`. */
function DoctorList({
  checks,
  onRun,
  ready,
}: {
  checks: readonly DoctorCheckView[];
  onRun: () => void;
  ready: boolean;
}) {
  const rows: readonly DoctorCheckView[] = checks;

  return (
    <div className="flex flex-col gap-[12px]">
      <h3 className="flex items-center gap-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
        <Stethoscope size={12} aria-hidden="true" />
        {strings.hub.doctorTitle}
      </h3>

      <div>
        {rows.map((check) => (
          <DoctorRow
            key={check.id}
            state={check.state}
            label={check.label}
            detail={check.detail}
            fix={check.fix}
          />
        ))}
      </div>

      <button
        type="button"
        className={BTN + ' ' + BTN_SECONDARY + ' self-start'}
        data-action="run-doctor"
        disabled={!ready}
        onClick={onRun}
      >
        {strings.settings.about.runDoctor}
      </button>
    </div>
  );
}

/** One check: a state icon, the label, the mono detail and the optional `Fix`. */
function DoctorRow({
  state,
  label,
  detail,
  fix,
}: {
  state: 'ok' | 'warn' | 'fail';
  label: string;
  detail: string;
  fix: string | null;
}) {
  const colour =
    state === 'ok' ? 'text-state-success' : state === 'warn' ? 'text-state-warning' : 'text-state-error';
  const edge =
    state === 'ok'
      ? 'border-l-state-success'
      : state === 'warn'
        ? 'border-l-state-warning'
        : 'border-l-state-error';

  return (
    <div
      className={
        'doctor-row mb-[6px] flex items-center gap-[12px] rounded-md border-l-[3px] bg-bg-raised px-[14px] py-[12px] ' +
        edge
      }
      data-doctor={label}
      data-state={state}
    >
      <span className={'grid h-[20px] w-[20px] shrink-0 place-items-center ' + colour} aria-hidden="true">
        {state === 'ok' ? <CircleCheckBig size={16} /> : <TriangleAlert size={16} />}
      </span>

      <span className="min-w-0 flex-1">
        <span className="block text-[12.5px] font-medium text-text-primary">{label}</span>
        <span className="mt-[2px] block font-mono text-[11px] text-text-muted">{detail}</span>
      </span>

      {fix === null ? null : (
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY}
          data-fix={fix}
          onClick={() => toast(strings.hub.doctorFixed(fix))}
        >
          {fix}
        </button>
      )}
    </div>
  );
}

/** A labelled form row, the shape the four flows share. */
function Field({
  label,
  required,
  children,
}: {
  label: string;
  required?: boolean;
  children: ReactNode;
}) {
  return (
    <label className="mb-[14px] block">
      <span className="mb-[5px] block text-[11.5px] font-medium text-text-secondary">
        {label}
        {required === true ? <span className="ml-[2px] text-state-error">*</span> : null}
      </span>
      {children}
    </label>
  );
}

/** The structured result line under a flow: idle, spinner, `OK · …`, or the failure. */
function ConnTest({ state, detail }: { state: 'idle' | 'busy' | 'ok' | 'fail'; detail: string }) {
  if (state === 'idle') {
    return null;
  }

  return (
    <div
      className={
        'conn-test flex items-center gap-[8px] rounded-md px-[12px] py-[9px] text-[12px] ' +
        (state === 'busy'
          ? 'bg-bg-base text-text-secondary'
          : state === 'ok'
            ? 'bg-green-subtle text-state-success'
            : 'bg-red-subtle text-state-error')
      }
      data-conn-state={state}
      role="status"
      aria-live="polite"
    >
      {state === 'busy' ? (
        <span className="spinner" aria-hidden="true" />
      ) : state === 'ok' ? (
        <CircleCheckBig size={16} aria-hidden="true" />
      ) : (
        <TriangleAlert size={16} aria-hidden="true" />
      )}
      <span className="result min-w-0 flex-1">{detail}</span>
    </div>
  );
}
