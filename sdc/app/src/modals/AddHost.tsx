import { useEffect, useState } from 'react';
import { Key, Laptop, Loader, Plug, Server, ShieldCheck } from 'lucide-react';

import { strings } from '../strings';
import { sdcpCall } from '../lib/sdcp';
import { addHost, hostKey, installHostKey, openTerminalForHost, runDoctor, trustHost } from '../store/intents';
import { useOverlayStore } from '../store/overlays';
import { useAppStore } from '../store/store';
import type { HostView } from '../store/types';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { toast } from '../store/toast';
import { Modal } from './Modal';

/**
 * `#addHostBd` - Add a host (spec section 9.12), with the trust step 0.7.13 added.
 *
 * Two shapes, one dialog: `Local` (this computer, always already connected) and `SSH / VPS`
 * (`user@host`), which reveals the target, the one-time password and the optional label.
 *
 * ## The second step
 *
 * Submitting is not the end of the flow any more, and that is the whole point of this release. The
 * daemon parses the target, **stores the port**, makes the key it owns, and then checks the machine's
 * host key against the pin SDC holds (`docs/REMOTE.md` §4):
 *
 *   * a key SDC already pinned → it connects, and this dialog closes;
 *   * a key SDC has never seen → the host arrives `untrusted` with its fingerprint and this dialog
 *     shows the card below, so a person decides. Nothing has been sent to that machine at this point -
 *     no key offered, no password typed - and only a `Trust and connect` click pins the key and lets
 *     the password be spent on the one-time install;
 *   * a key that is **not** the pinned one → the host goes `offline` with both fingerprints named, and
 *     the dialog shows that sentence instead of a button. There is deliberately no "continue anyway".
 *
 * The status comes from the event log (`HostStatus`, folded in `store/store.ts`) rather than from this
 * component: whether a machine can be reached is the daemon's fact, and a dialog keeping its own copy
 * would be a second answer to the same question (spec section 3.3).
 */
export function AddHost() {
  const open = useOverlayStore((state) => state.addHostOpen);
  const close = useOverlayStore((state) => state.closeAddHost);
  /* The host this dialog is *about*, when it was opened from the switcher to answer a question about a
     host rather than to add one (0.7.13): the same card, no form. */
  const openFor = useOverlayStore((state) => state.addHostHostId);
  const [type, setType] = useState<'local' | 'ssh'>('local');
  const [target, setTarget] = useState('');
  const [label, setLabel] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  /** The host this dialog is watching, once `host.add` has answered with one - or the one it opened on. */
  const [pending, setPending] = useState<{ hostId: string; label: string } | null>(null);
  const [trusting, setTrusting] = useState(false);
  /** A key install is in flight (0.7.13): the one call that spends the password. */
  const [installing, setInstalling] = useState(false);
  /**
   * SDC's **public** key, for the card's manual line. Only the public half is ever read, and only so a
   * host that requires a verification code can be finished by hand instead of being a dead end.
   */
  const [sdcKey, setSdcKey] = useState<string | null>(null);
  /** What `host.key` answered for the host in front of the user (0.7.13), if it has been asked. */
  const [scanned, setScanned] = useState<{ hostKey: string; keyType: string; matches: boolean | null; pinnedKey: string | null } | null>(
    null,
  );

  const host: HostView | undefined = useAppStore((state) =>
    pending === null ? undefined : state.hosts.find((candidate) => candidate.id === pending.hostId),
  );
  /** The host's own environment checks, when this dialog is about a host and they have been asked. */
  const doctor = useAppStore((state) => (pending === null ? undefined : state.doctor[pending.hostId]));

  /**
   * The state this card exists for after the pin: the daemon says the one thing left is the **key
   * install** (`host.doctor`'s `ssh` row carries `fix: "Install key"`, and it is derived from the trust
   * state there - a pin that is in place and a probe that still failed).
   *
   * Reading the *daemon's* fix rather than guessing from `status`/`pinned` here is what keeps the two
   * surfaces from disagreeing: there is one place that decides when a password is the next step, and this
   * card only renders what it is told.
   */
  const needsKeyInstall =
    openFor !== null && openFor !== 'local' && (doctor?.some((row) => row.fix === 'Install key') ?? false);

  /* SDC's public key, for the manual line under the button: asked once, and only when there is a use for
     it. `ssh.key` is a read, so opening the card creates nothing. */
  useEffect(() => {
    if (!needsKeyInstall || sdcKey !== null) {
      return;
    }

    void sdcpCall('ssh.key', {}).then(
      (answer) => setSdcKey(answer.publicKey),
      () => setSdcKey(null),
    );
  }, [needsKeyInstall, sdcKey]);

  const reset = (): void => {
    setTarget('');
    setLabel('');
    setPassword('');
    setType('local');
    setPending(null);
    setTrusting(false);
    setScanned(null);
  };

  /*
   * Opened about a host (the switcher's "Keys & doctor", or a `needs your trust` row): there is nothing
   * to fill in, so the dialog goes straight to the card - and it **asks** for the fingerprint
   * (`host.key`), because the event that carried it may have been pushed to a window that is no longer
   * open. That is the relaunch case, and it is a call rather than a guess.
   */
  useEffect(() => {
    if (!open || openFor === null) {
      return;
    }

    const name = useAppStore.getState().hosts.find((candidate) => candidate.id === openFor)?.name ?? openFor;

    setPending({ hostId: openFor, label: name });
    /* `local` is this machine: its environment is worth checking (`runDoctor`) and it has no host key to
       pin, so `host.key` is not asked - a call that could only answer with an error. */
    if (openFor !== 'local') {
      void hostKey(openFor).then((answer) => {
        if (answer !== null) {
          setScanned(answer);
        }
      });
    }
    void runDoctor(openFor);
  }, [open, openFor]);

  /* A host that reached a verdict - while *adding* it - closes the dialog: the row is on the list, its own
     line carries the sentence, and there is nothing left for a modal to say. A dialog opened *about* a
     host stays where it is, because the person came here to look at it. */
  useEffect(() => {
    if (openFor !== null || pending === null || host === undefined) {
      return;
    }

    if (host.status === 'connected' || host.status === 'offline') {
      close();
      reset();
    }
  }, [openFor, pending, host, close]);

  const submit = (): void => {
    setBusy(true);

    void addHost({
      type,
      target: target.trim(),
      label: label.trim(),
      ...(password === '' ? {} : { password }),
    }).then((answer) => {
      setBusy(false);

      /* A refused call and a cancel leave the dialog as it was; the password is in the field still. */
      if (answer === null) {
        return;
      }

      if (answer.hostId === 'local') {
        close();
        reset();

        return;
      }

      setPending({ hostId: answer.hostId, label: label.trim() === '' ? target.trim() : label.trim() });
    });
  };

  /**
   * The fingerprint in play: what `host.key` just answered, or the one the `HostStatus` for this host
   * carried. Both are the *same* value in the add flow; the difference is only whether this window was
   * open when it was pushed - which is the whole reason `host.key` exists.
   */
  const fingerprint = scanned?.hostKey !== undefined && scanned.hostKey !== '' ? scanned.hostKey : (host?.hostKey ?? '');
  /** The host presents a key that is **not** the pinned one: `host.key` said so, or the row is offline. */
  const rePin = scanned !== null && scanned.matches === false;
  /** There is a question to answer: nothing is pinned, or the pin no longer matches. */
  const needsTrust = rePin || host?.status === 'untrusted' || (scanned !== null && scanned.matches === null);

  const trust = (): void => {
    if (pending === null || fingerprint === '') {
      return;
    }

    setTrusting(true);

    /* The password from the field is re-sent here, and only here: it is spent after the pin lands. */
    void trustHost(pending.hostId, fingerprint, password).then((trusted) => {
      setTrusting(false);

      /* Ask again so the card can say `pinned` rather than leaving the old verdict on screen - and on a
         re-pin the answer is also what the host's own row will show. */
      if (trusted) {
        void hostKey(pending.hostId).then((answer) => {
          if (answer !== null) {
            setScanned(answer);
          }
        });
      }
    });
  };

  /* Closing by hand (the ×, the backdrop, Escape, Cancel): the dialog forgets its own state, because
     the host is the daemon's row now and this component must not remember a half-finished flow. */
  const shutdown = (): void => {
    close();
    reset();
  };

  return (
    <Modal open={open} label={strings.addHost.title} onClose={shutdown} center className="addhost-dlg">
      <div className="flex items-start gap-[12px] border-b border-border-subtle px-[18px] py-[16px]">
        <div className="grid h-[34px] w-[34px] shrink-0 place-items-center rounded-md bg-accent-subtle text-accent">
          <Server size={18} aria-hidden="true" />
        </div>
        <div>
          <div className="text-[14px] font-semibold text-text-primary">{strings.addHost.title}</div>
          <div className="mt-[2px] text-[12px] text-text-muted">{strings.addHost.sub}</div>
        </div>
      </div>

      <div className="px-[18px] py-[16px]">
        {/* A dialog about a host has nothing to fill in (0.7.13): the card and the host's own
            environment are the whole content, and a form for *adding* a host would be a lie about what
            this surface is doing. */}
        {openFor === null ? (
          <>
        <div className="flex gap-[8px]">
          {(['local', 'ssh'] as const).map((option) => {
            const Icon = option === 'local' ? Laptop : Server;
            const selected = option === type;

            return (
              <button
                key={option}
                type="button"
                data-host-type={option}
                aria-pressed={selected}
                className={
                  'host-type-option flex flex-1 flex-col items-start gap-[4px] rounded-md border px-[12px] py-[10px] text-left transition-all duration-fast ease-ease ' +
                  (selected
                    ? 'selected border-accent bg-accent-subtle'
                    : 'border-border-default bg-bg-raised hover:border-border-strong')
                }
                onClick={() => setType(option)}
              >
                <span className="flex items-center gap-[6px] text-[12.5px] font-medium text-text-primary">
                  <Icon size={16} aria-hidden="true" />
                  {strings.addHost.types[option].label}
                </span>
                <span className="font-mono text-[10.5px] text-text-muted">
                  {strings.addHost.types[option].desc}
                </span>
              </button>
            );
          })}
        </div>

        {type === 'ssh' ? (
          <div className="mt-[14px] flex flex-col gap-[12px]">
            <label className="flex flex-col gap-[5px]">
              <span className="text-[11.5px] font-medium text-text-secondary">
                {strings.addHost.sshTarget}
              </span>
              <input
                type="text"
                id="sshTarget"
                className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
                placeholder={strings.addHost.sshTargetPlaceholder}
                value={target}
                onChange={(event) => setTarget(event.target.value)}
              />
              <span className="text-[11px] text-text-muted">{strings.addHost.sshTargetHelp}</span>
            </label>

            <label className="flex flex-col gap-[5px]">
              <span className="text-[11.5px] font-medium text-text-secondary">{strings.addHost.password}</span>
              <input
                type="password"
                id="sshPassword"
                autoComplete="off"
                className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
                placeholder={strings.addHost.passwordPlaceholder}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
              <span className="text-[11px] text-text-muted">{strings.addHost.passwordHelp}</span>
            </label>

            <label className="flex flex-col gap-[5px]">
              <span className="text-[11.5px] font-medium text-text-secondary">
                {strings.addHost.labelField}
              </span>
              <input
                type="text"
                id="sshLabel"
                className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
                placeholder={strings.addHost.labelPlaceholder}
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </label>
          </div>
        ) : null}
          </>
        ) : null}
      </div>

      {/*
        The trust step (0.7.13).

        The card exists only once `host.add` has answered with a host id (`pending`), and it shows
        whatever the log currently says about that host: `connecting` while the scan runs, `untrusted`
        when a fingerprint needs a decision, and `offline` with the daemon's own sentence when the key
        changed or the machine refused. The fingerprint is the exact string `host.trust` takes back,
        which is why it is a field on the event rather than something read out of the paragraph above it.
      */}
      {pending === null ? null : (
        <div className="border-t border-border-subtle px-[18px] py-[14px]" data-ssh-pending={pending.hostId}>
          <div className="flex items-start gap-[10px]">
            <div className="grid h-[28px] w-[28px] shrink-0 place-items-center rounded-md bg-accent-subtle text-accent">
              {host?.status === 'untrusted' && !trusting ? (
                <ShieldCheck size={15} aria-hidden="true" />
              ) : (
                <Loader size={15} aria-hidden="true" className="animate-spin" />
              )}
            </div>

            <div className="min-w-0 flex-1">
              <div className="text-[12.5px] font-semibold text-text-primary">
                {rePin
                  ? strings.addHost.trust.rePinTitle(pending.label)
                  : host?.status === 'untrusted' || (scanned !== null && scanned.matches === null)
                    ? strings.addHost.trust.title(pending.label)
                    : strings.addHost.trust.waiting}
              </div>

              {/* The daemon's own sentence: what it is doing, or what is wrong. */}
              <p className="mt-[3px] text-[11.5px] leading-[1.5] text-text-secondary">
                {host?.detail === undefined || host.detail === ''
                  ? strings.addHost.connecting(pending.label)
                  : host.detail}
              </p>

              {/*
                The two things a person can decide, and the state that is a fact rather than a question:
                `untrusted`/`changed` show the fingerprint and the button; a pinned key says so quietly.
              */}
              {needsTrust && fingerprint !== '' ? (
                <div className="mt-[10px] flex flex-col gap-[6px]">
                  <span className="text-[10.5px] font-semibold uppercase tracking-wide text-text-muted">
                    {strings.addHost.trust.fingerprint}
                  </span>
                  <code
                    className="ssh-fingerprint break-all rounded-sm border border-border-subtle bg-bg-input px-[8px] py-[6px] font-mono text-[11px] text-text-primary"
                    data-ssh-fingerprint={fingerprint}
                  >
                    {fingerprint}
                  </code>

                  {rePin && scanned?.pinnedKey !== null && scanned?.pinnedKey !== undefined && scanned.pinnedKey !== '' ? (
                    <p
                      className="text-[11px] leading-[1.5] text-state-warning"
                      data-ssh-pinned-before={scanned.pinnedKey}
                    >
                      {strings.addHost.trust.wasPinned(scanned.pinnedKey)}
                    </p>
                  ) : null}

                  <p className="text-[11px] leading-[1.5] text-text-muted">{strings.addHost.trust.compare}</p>

                  <div className="mt-[2px] flex items-center gap-[8px]">
                    <button
                      type="button"
                      className={BTN + ' ' + BTN_PRIMARY}
                      id="sshTrustBtn"
                      disabled={trusting}
                      onClick={trust}
                    >
                      {trusting ? <span className="spinner" aria-hidden="true" /> : <ShieldCheck size={12} aria-hidden="true" />}
                      {trusting
                        ? strings.addHost.trust.accepting
                        : rePin
                          ? strings.addHost.trust.rePin
                          : strings.addHost.trust.accept}
                    </button>
                  </div>
                </div>
              ) : null}

              {/* This machine: there is no host key to pin, and saying so is better than an empty card. */}
              {openFor !== null && openFor === 'local' ? (
                <p className="mt-[6px] text-[11px] leading-[1.5] text-text-muted" data-ssh-key-state="local">
                  {strings.addHost.trust.localKey}
                </p>
              ) : null}

              {/*
                The step after the pin, and the reason a host can look *added* and never connect (0.7.13).

                The fingerprint above is pinned and the machine still will not let SDC in, because the far
                side has no reason to: its `authorized_keys` does not carry SDC's key yet. The one thing
                that fixes it is the password, once - and until this block existed there was nowhere to
                type it for a host that was already added, so the host stayed red for ever and the only way
                out was to re-add it by hand and hope the form was filled in the right order.
              */}
              {openFor !== null && openFor !== 'local' && needsKeyInstall ? (
                <div className="mt-[10px] flex flex-col gap-[8px]" data-ssh-key-install={pending.hostId}>
                  <div className="text-[12px] font-semibold text-text-primary">
                    {strings.addHost.keyInstall.title(pending.label)}
                  </div>
                  <p className="text-[11px] leading-[1.5] text-text-secondary">{strings.addHost.keyInstall.sub}</p>

                  <label className="flex flex-col gap-[5px]">
                    <span className="text-[11px] font-medium text-text-secondary">
                      {strings.addHost.keyInstall.password}
                    </span>
                    <input
                      type="password"
                      id="sshKeyInstallPassword"
                      autoComplete="off"
                      className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
                      placeholder={strings.addHost.keyInstall.passwordPlaceholder}
                      value={password}
                      onChange={(event) => setPassword(event.target.value)}
                    />
                  </label>

                  <div className="flex items-center gap-[8px]">
                    <button
                      type="button"
                      className={BTN + ' ' + BTN_PRIMARY}
                      id="sshKeyInstallBtn"
                      disabled={installing || password === ''}
                      onClick={() => {
                        setInstalling(true);

                        void installHostKey(pending.hostId, password).then(() => {
                          setInstalling(false);
                          setPassword('');
                          /* The daemon measured the host; ask it again so the card and the rows move to
                             the truth (`connected`, or the sentence that says what the host refused). */
                          void runDoctor(pending.hostId);
                        });
                      }}
                    >
                      {installing ? (
                        <span className="spinner" aria-hidden="true" />
                      ) : (
                        <Key size={12} aria-hidden="true" />
                      )}
                      {installing ? strings.addHost.keyInstall.installing : strings.addHost.keyInstall.button}
                    </button>
                  </div>

                  {/* A host that only offers a verification code cannot be finished from here, and this
                      is the one line that is enough to do it by hand. */}
                  <p className="text-[11px] leading-[1.5] text-text-muted">
                    {strings.addHost.keyInstall.manual}
                  </p>
                  {sdcKey === null ? null : (
                    <code className="break-all rounded-sm border border-border-subtle bg-bg-input px-[8px] py-[6px] font-mono text-[10.5px] text-text-secondary">
                      {strings.addHost.keyInstall.publicKey(sdcKey)}
                    </code>
                  )}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      )}

      <div className="flex items-center gap-[8px] border-t border-border-subtle px-[18px] py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={shutdown}>
          {strings.addHost.cancel}
        </button>

        <div className="flex-1" />

        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          id="addHostSubmit"
          disabled={busy || pending !== null}
          onClick={submit}
        >
          {busy ? <span className="spinner" aria-hidden="true" /> : <Plug size={12} aria-hidden="true" />}
          {strings.addHost.connect}
        </button>
      </div>
      {/* The host's own environment (0.7.13): `host.doctor` about *that* machine - its key, its CLIs,
          its home, and the chat's folder when a chat named one. The `Trust`/`Re-pin` fixes are the ones
          this dialog can act on, and they land on the card above. */}
      {pending === null || doctor === undefined || doctor.length === 0 ? null : (
        <div className="border-t border-border-subtle px-[18px] py-[12px]" data-host-doctor={pending.hostId}>
          <div className="mb-[8px] flex items-center gap-[8px]">
            <span className="text-[11px] font-semibold uppercase tracking-wide text-text-muted">
              {strings.addHost.doctorTitle}
            </span>
            <button
              type="button"
              className="ml-auto rounded-sm px-[6px] py-[2px] text-[10.5px] text-text-muted transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary"
              onClick={() => void runDoctor(pending.hostId)}
            >
              {strings.settings.about.runDoctor}
            </button>
          </div>

          <div className="flex max-h-[180px] flex-col gap-[4px] overflow-y-auto">
            {doctor.map((check) => (
              <div
                key={check.id}
                className="ssh-doctor-row flex items-center gap-[8px] text-[11px]"
                data-doctor={check.id}
                data-doctor-state={check.state}
              >
                <span
                  className={
                    'h-[6px] w-[6px] shrink-0 rounded-full ' +
                    (check.state === 'ok' ? 'bg-state-success' : check.state === 'warn' ? 'bg-state-waiting' : 'bg-state-error')
                  }
                  aria-hidden="true"
                />
                <span className="shrink-0 text-text-secondary">{check.label}</span>
                <span
                  className="min-w-0 flex-1 truncate text-right font-mono text-[10.5px] text-text-muted"
                  title={check.detail}
                >
                  {check.detail}
                </span>

                {check.fix === undefined ? null : (
                  <button
                    type="button"
                    className="shrink-0 rounded-sm border border-border-subtle px-[5px] py-[1px] text-[10.5px] text-text-secondary transition-colors duration-fast ease-ease hover:border-border-default hover:text-text-primary"
                    data-doctor-fix={check.fix}
                    onClick={() => {
                      const fix = check.fix;

                      if (fix === undefined || fix === null) {
                        return;
                      }

                      /*
                       * `Trust` and `Re-pin` are the two fixes this surface carries out, because this *is*
                       * the trust surface: both mean "ask what it presents now", which is `host.key`, and
                       * the card above renders the answer.
                       *
                       * Everything else tells the truth (0.7.13). A fix button that re-scans on `Install`
                       * is a button doing something unrelated to its own label, which is the bug shape this
                       * release removes - so `Install` opens the **Terminal on this host** (the person runs
                       * the installer themselves; SDC does not install software on somebody's server), and
                       * any other label says where to do it.
                       */
                      if (fix !== 'Trust' && fix !== 'Re-pin') {
                        if (fix === 'Install') {
                          /* The card closes first: the Terminal lives in the right panel, behind this modal -
                             and `openTerminalForHost` focuses one of that host's chats, so the command runs
                             in *that* folder on *that* machine. */
                          useOverlayStore.getState().closeAddHost();
                          openTerminalForHost(pending.hostId);
                          return;
                        }

                        if (fix === 'Install key') {
                          /* The password field that finishes this *is on this card* (see the block below):
                             the button's job is to put the caret in it, not to open another surface. */
                          document.getElementById('sshKeyInstallPassword')?.focus();
                          return;
                        }

                        toast(strings.hub.doctorFixManual(fix));

                        return;
                      }

                      void hostKey(pending.hostId).then((answer) => {
                        if (answer !== null) {
                          setScanned(answer);
                        }
                      });
                    }}
                  >
                    {check.fix}
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

    </Modal>
  );
}
