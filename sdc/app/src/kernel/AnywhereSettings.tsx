import { useCallback, useEffect, useState } from 'react';
import QRCode from 'qrcode';

import type { AnywhereDevice, AnywherePairRequest, AnywhereStatus } from '../../../protocol/types';
import { strings } from '../strings';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';
import { anywhereStatus, beginPairing, configureAnywhere, confirmPairing, devices as listDevices, pairRequests, resetAnywhere, revokeDevice, setAnywhere } from '../store/anywhere';
import { toast } from '../store/toast';
import { copyText } from '../lib/external';

const FIELD =
  'h-[30px] rounded-md border border-border-default bg-bg-base px-[10px] text-[12.5px] text-text-primary focus:border-border-strong focus:outline-none';

interface Offer {
  url: string;
  fingerprint: string;
  expiresAt: number;
  guest: boolean;
  qr: string;
}

/**
 * Settings → SDC Anywhere (0.17): the one place the feature is turned on, devices are paired and revoked, and
 * the limits are set. A pairing request from a phone shows its six digits here; the person confirms only when
 * they match the phone's. The browser can do none of this: the daemon refuses every `anywhere.*` call that
 * does not come from this window.
 */
export function AnywhereSettings() {
  const words = strings.anywhere;
  const [status, setStatus] = useState<AnywhereStatus | null>(null);
  const [list, setList] = useState<AnywhereDevice[]>([]);
  const [requests, setRequests] = useState<AnywherePairRequest[]>([]);
  const [offer, setOffer] = useState<Offer | null>(null);

  const refresh = useCallback(async () => {
    const [next, devices, pending] = await Promise.all([anywhereStatus(), listDevices(), pairRequests()]);

    setStatus(next);
    setList(devices);
    setRequests(pending);
  }, []);

  useEffect(() => {
    void refresh();

    const timer = setInterval(() => void refresh(), 2000);

    return () => clearInterval(timer);
  }, [refresh]);

  if (status === null) {
    return (
      <section className="mb-[20px]">
        <h3 className="text-[14px] font-semibold text-text-primary">{words.title}</h3>
        <p className="mt-[2px] text-[12px] text-text-muted">{words.intro}</p>
      </section>
    );
  }

  const link = !status.running ? words.link.off : status.connected ? words.link.connected : status.lastError ? words.link.error(status.lastError) : words.link.connecting;
  const startPairing = async (guest: boolean): Promise<void> => {
    const begun = await beginPairing(guest);

    if (begun) setOffer({ ...begun, guest, qr: await QRCode.toDataURL(begun.url, { margin: 1, width: 232, errorCorrectionLevel: 'M' }) });
  };
  const number = (label: string, value: number, key: 'approvalTimeoutSec' | 'viewIdleLockMinutes' | 'operateWindowMinutes' | 'guestSessionMaxMinutes', scale = 1) => (
    <label className="mt-[10px] flex items-center justify-between gap-[12px] text-[12.5px] text-text-primary">
      {label}
      <input
        type="number"
        min={1}
        className={FIELD + ' w-[78px] font-mono'}
        defaultValue={Math.round(value / scale)}
        onBlur={(event) => {
          const parsed = Number.parseInt(event.target.value, 10);

          if (Number.isFinite(parsed) && parsed > 0 && parsed !== Math.round(value / scale)) {
            void configureAnywhere({ [key]: parsed * scale }).then((next) => next && setStatus(next));
          }
        }}
      />
    </label>
  );

  return (
    <section className="mb-[20px]">
      <h3 className="text-[14px] font-semibold text-text-primary">{words.title}</h3>
      <p className="mt-[2px] text-[12px] text-text-muted">{words.intro}</p>

      <div className="mt-[14px] flex items-center justify-between gap-[12px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px]">
        <div className="text-[12.5px]">
          <div className="font-semibold text-text-primary">{status.running ? words.on : words.off}</div>
          <div className="text-text-muted" role="status">{link}</div>
          {status.running ? (
            <div className="text-text-muted">
              {words.browsers(status.browserConnections)} · {words.waiting(status.waitingApprovals)}
            </div>
          ) : null}
        </div>
        <button
          type="button"
          className={BTN + ' ' + (status.running ? BTN_SECONDARY : BTN_PRIMARY)}
          onClick={() => void setAnywhere(!status.running).then((next) => (next ? setStatus(next) : refresh()))}
        >
          {status.running ? words.disable : words.enable}
        </button>
      </div>

      <h4 className="mt-[16px] text-[13px] font-semibold text-text-primary">{words.keys.title}</h4>
      <p className="text-[12px] text-text-muted">{status.keyStorage.backend === 'os' ? words.keys.os : words.keys.file}</p>
      {status.keyStorage.backend !== 'os' ? (
        <label className="mt-[6px] flex items-start gap-[8px] text-[12.5px] text-text-primary">
          <input
            type="checkbox"
            checked={status.keyStorage.acceptedFileKey}
            onChange={(event) => void configureAnywhere({ acceptFileKey: event.target.checked }).then((next) => next && setStatus(next))}
          />
          <span>
            {words.keys.accept}
            <span className="block text-[11.5px] text-text-muted">{words.keys.acceptHelp}</span>
          </span>
        </label>
      ) : null}

      {requests.map((request) => (
        <div key={request.deviceId} role="alertdialog" aria-label={words.pair.request(request.name)} className="mt-[14px] rounded-md border border-accent bg-accent-subtle px-[12px] py-[10px]">
          <div className="text-[13px] font-semibold text-text-primary">{words.pair.request(request.name)}{request.guest ? ` · ${words.devices.guest}` : ''}</div>
          <div className="text-[12px] text-text-muted">{words.pair.compare}</div>
          <div className="my-[6px] font-mono text-[26px] tracking-[0.15em] text-text-primary">{request.code}</div>
          <div className="flex gap-[8px]">
            <button type="button" className={BTN + ' ' + BTN_PRIMARY + ' ' + BTN_SM} onClick={() => void confirmPairing(request.deviceId, true).then(refresh)}>
              {words.pair.accept}
            </button>
            <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_SM} onClick={() => void confirmPairing(request.deviceId, false).then(refresh)}>
              {words.pair.decline}
            </button>
          </div>
        </div>
      ))}

      <h4 className="mt-[16px] text-[13px] font-semibold text-text-primary">{words.devices.title}</h4>
      {list.length === 0 ? <p className="text-[12px] text-text-muted">{words.devices.none}</p> : null}
      <ul>
        {list.map((device) => (
          <li key={device.id} className="mt-[6px] flex items-center justify-between gap-[10px] text-[12.5px] text-text-primary">
            <span>
              {device.name}
              {device.guest ? <span className="ml-[6px] text-text-muted">{words.devices.guest}</span> : null}
              {device.revokedAt !== null ? <span className="ml-[6px] text-state-error">{words.devices.revoked}</span> : null}
              <span className="block text-[11.5px] text-text-muted">{device.lastSeen === null ? words.devices.never : words.devices.lastSeen(new Date(device.lastSeen).toLocaleString())}</span>
            </span>
            {device.revokedAt === null ? (
              <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_SM} onClick={() => void revokeDevice(device.id).then(refresh)}>
                {words.devices.revoke}
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      <div className="mt-[10px] flex gap-[8px]">
        <button type="button" className={BTN + ' ' + BTN_PRIMARY + ' ' + BTN_SM} disabled={!status.running} onClick={() => void startPairing(false)}>
          {words.devices.add}
        </button>
        <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_SM} disabled={!status.running} onClick={() => void startPairing(true)}>
          {words.devices.addGuest}
        </button>
      </div>

      {offer !== null ? (
        <div role="dialog" aria-label={words.pair.title} className="mt-[14px] rounded-md border border-border-default bg-bg-raised px-[14px] py-[12px]">
          <div className="text-[13px] font-semibold text-text-primary">{words.pair.title}{offer.guest ? ` · ${words.devices.guest}` : ''}</div>
          <div className="text-[12px] text-text-muted">{words.pair.scan}</div>
          <img src={offer.qr} alt="" width={232} height={232} className="my-[8px] rounded-md bg-white" />
          <div className="text-[11.5px] text-text-muted">{words.pair.expires(Math.max(1, Math.round((offer.expiresAt - Date.now()) / 60000)))}</div>
          <div className="mt-[4px] text-[11.5px] text-text-muted">{words.pair.fingerprint}: <span className="font-mono text-text-primary">{offer.fingerprint}</span></div>
          <div className="mt-[8px] flex gap-[8px]">
            <button
              type="button"
              className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_SM}
              onClick={() => void copyText(offer.url).then(() => toast(words.pair.copied))}
            >
              {words.pair.copy}
            </button>
            <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_SM} onClick={() => setOffer(null)}>
              {words.pair.close}
            </button>
          </div>
        </div>
      ) : null}

      <h4 className="mt-[16px] text-[13px] font-semibold text-text-primary">{words.settings.title}</h4>
      <label className="mt-[6px] flex items-center justify-between gap-[12px] text-[12.5px] text-text-primary">
        {words.settings.notifyWhen}
        <select
          className={FIELD}
          value={status.settings.notifyWhen}
          onChange={(event) => void configureAnywhere({ notifyWhen: event.target.value as 'idle' | 'always' | 'never' }).then((next) => next && setStatus(next))}
        >
          {(['idle', 'always', 'never'] as const).map((id) => (
            <option key={id} value={id}>{words.settings.notifyOptions[id]}</option>
          ))}
        </select>
      </label>
      <label className="mt-[10px] flex items-center justify-between gap-[12px] text-[12.5px] text-text-primary">
        {words.settings.onTimeout}
        <select
          className={FIELD}
          value={status.settings.onTimeout}
          onChange={(event) => void configureAnywhere({ onTimeout: event.target.value as 'pause' | 'deny' }).then((next) => next && setStatus(next))}
        >
          {(['pause', 'deny'] as const).map((id) => (
            <option key={id} value={id}>{words.settings.onTimeoutOptions[id]}</option>
          ))}
        </select>
      </label>
      {number(words.settings.timeout, status.settings.approvalTimeoutSec, 'approvalTimeoutSec', 60)}
      {number(words.settings.viewLock, status.settings.viewIdleLockMinutes, 'viewIdleLockMinutes')}
      {number(words.settings.operate, status.settings.operateWindowMinutes, 'operateWindowMinutes')}
      {number(words.settings.guest, status.settings.guestSessionMaxMinutes, 'guestSessionMaxMinutes')}
      <label className="mt-[10px] flex items-center justify-between gap-[12px] text-[12.5px] text-text-primary">
        {words.settings.email}
        <input
          type="email"
          className={FIELD + ' w-[220px]'}
          placeholder={words.settings.emailPlaceholder}
          maxLength={254}
          autoComplete="email"
          defaultValue={status.settings.email}
          onBlur={(event) => {
            const next = event.target.value.trim();

            if (next !== status.settings.email) {
              void configureAnywhere({ email: next }).then((saved) => (saved ? setStatus(saved) : (event.target.value = status.settings.email)));
            }
          }}
        />
      </label>
      <p className="mt-[4px] text-[11.5px] text-text-muted">{words.settings.emailHelp}</p>
      <label className="mt-[10px] flex items-center justify-between gap-[12px] text-[12.5px] text-text-primary">
        {words.settings.escalate}
        <input
          type="number"
          min={1}
          max={3600}
          className={FIELD + ' w-[78px] font-mono'}
          defaultValue={status.settings.escalateEmailSec}
          onBlur={(event) => {
            const parsed = Number.parseInt(event.target.value, 10);

            if (Number.isFinite(parsed) && parsed >= 1 && parsed <= 3600 && parsed !== status.settings.escalateEmailSec) {
              void configureAnywhere({ escalateEmailSec: parsed }).then((saved) => saved && setStatus(saved));
            }
          }}
        />
      </label>
      <p className="mt-[6px] text-[11.5px] text-text-muted">{words.settings.note}</p>

      <h4 className="mt-[16px] text-[13px] font-semibold text-text-primary">{words.danger.title}</h4>
      <p className="text-[12px] text-text-muted">{words.danger.resetHelp}</p>
      <button
        type="button"
        className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_SM + ' mt-[6px]'}
        onClick={() => {
          if (window.confirm(words.danger.resetConfirm)) void resetAnywhere().then(refresh);
        }}
      >
        {words.danger.reset}
      </button>
    </section>
  );
}
