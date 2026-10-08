import { useEffect, useRef, useState } from 'react';
import type { Model } from '../state/model';
import { ApprovalCard } from './ApprovalCard';
import { Chat } from './Chat';
import { Files } from './Files';
import { ModelContext, useModel, useSnapshot, useT } from './hooks';

type Tab = 'inbox' | 'chat' | 'files' | 'more';

export function App({ model }: { model: Model }) {
  return (
    <ModelContext.Provider value={model}>
      <Shell />
    </ModelContext.Provider>
  );
}

function Shell() {
  const snap = useSnapshot();
  const t = useT();

  if (snap.phase === 'loading') return <Centered>{t('loading')}</Centered>;
  if (snap.phase === 'pairing') return <PairScreen />;
  if (snap.phase === 'signin') return <SignInScreen />;
  if (snap.phase === 'unpaired') return <Welcome />;

  return <Home />;
}

function Centered({ children }: { children: React.ReactNode }) {
  return <main className="centered">{children}</main>;
}

// --- unpaired -------------------------------------------------------------------------------------------

function Welcome() {
  const t = useT();
  const snap = useSnapshot();

  return (
    <Centered>
      <img className="hero-logo" src="/icon-192.png" alt="" width="84" height="84" />
      <h1>{t('welcomeTitle')}</h1>
      <p className="muted">{t('welcomeBody')}</p>
      {snap.error && <p role="alert" className="error">{snap.error}</p>}
      <RejoinPanel />
      <MailLinkForm />
    </Centered>
  );
}

/** The browser lost its keys (data cleared) but the phone still has the passkey: one fingerprint brings the pairing back. */
function RejoinPanel() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();

  return (
    <section className="panel" aria-labelledby="rejoin-title">
      <h2 id="rejoin-title">{t('rejoinTitle')}</h2>
      <p className="muted">{t('rejoinBody')}</p>
      <button className="primary" onClick={() => void model.recoverWithPasskey()} disabled={snap.recovering}>{snap.recovering ? t('rejoinWorking') : t('rejoinButton')}</button>
    </section>
  );
}

/** "New browser, or lost your phone?": the way back in when no paired device is at hand. The reply never says whether the address exists. */
function MailLinkForm() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();
  const [email, setEmail] = useState('');
  const sending = snap.mailLink === 'sending';
  const notice = snap.mailLink === 'sent' ? t('mailSent') : snap.mailLink === 'invalid' ? t('mailInvalid') : snap.mailLink === 'busy' ? t('mailBusy') : snap.mailLink === 'error' ? t('mailError') : null;

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();
        void model.requestLink(email);
      }}
    >
      <h2>{t('mailTitle')}</h2>
      <p className="muted">{t('mailBody')}</p>
      <label htmlFor="mail-address">{t('mailLabel')}</label>
      <input id="mail-address" type="email" value={email} onChange={(event) => setEmail(event.target.value)} maxLength={254} autoComplete="email" required disabled={sending} />
      <button type="submit" disabled={sending || email.trim() === ''}>{sending ? t('mailSending') : t('mailSend')}</button>
      {notice && <p role={snap.mailLink === 'sent' ? 'status' : 'alert'} className={snap.mailLink === 'sent' ? 'muted' : 'error'}>{notice}</p>}
    </form>
  );
}

/** The page a sign-in link opens. Opening it did nothing; this button is what uses the link. */
function SignInScreen() {
  const t = useT();
  const model = useModel();
  const signin = useSnapshot().signin;

  if (!signin) return null;

  const working = signin.step === 'working';
  const problem: Record<string, string> = {
    invalid: t('signinInvalid'),
    offline: t('signinOffline'),
    refused: t('signinRefused'),
    busy: t('signinBusy'),
    error: t('signinError'),
  };

  return (
    <Centered>
      <img className="hero-logo" src="/icon-192.png" alt="" width="84" height="84" />
      <h1>{t('signinTitle')}</h1>
      <p className="muted">{t('signinBody')}</p>
      <button className="primary" onClick={() => void model.redeemLink()} disabled={working || signin.step === 'invalid'}>
        {working ? t('signinWorking') : t('signinContinue')}
      </button>
      {problem[signin.step] && <p role="alert" className="error">{problem[signin.step]}</p>}
    </Centered>
  );
}

function PairScreen() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();
  const [name, setName] = useState(() => guessDeviceName());
  const [guest, setGuest] = useState(false);
  const busy = snap.progress !== null;

  return (
    <main className="page">
      <h1>{t('pairTitle')}</h1>
      <section className="panel">
        <h2>{t('pairFingerprint')}</h2>
        <p className="mono big">{snap.offer?.fingerprint}</p>
        <p className="muted">{t('moreFingerprintHelp')}</p>
      </section>

      {snap.progress?.step === 'confirm' ? (
        <section className="panel" aria-live="polite">
          <h2>{t('pairConfirm')}</h2>
          <p className="code" aria-label={snap.progress.code.replace(' ', ', ')}>{snap.progress.code}</p>
          <p className="muted">{t('pairConfirmBody')}</p>
        </section>
      ) : (
        <form
          className="panel"
          onSubmit={(event) => {
            event.preventDefault();
            void model.startPairing(name.trim() || 'Browser', guest);
          }}
        >
          <label htmlFor="device-name">{t('pairName')}</label>
          <input id="device-name" value={name} onChange={(event) => setName(event.target.value)} maxLength={60} autoComplete="off" disabled={busy} />
          <label className="check">
            <input type="checkbox" checked={guest} onChange={(event) => setGuest(event.target.checked)} disabled={busy} />
            <span>{t('pairGuest')}</span>
          </label>
          <button type="submit" className="primary" disabled={busy}>
            {snap.progress?.step === 'passkey' ? t('pairPasskey') : snap.progress ? t('pairConnecting') : t('pairStart')}
          </button>
        </form>
      )}

      {snap.error && (
        <p role="alert" className="error">
          {snap.error}
        </p>
      )}
    </main>
  );
}

function guessDeviceName(): string {
  const ua = navigator.userAgent;
  const os = /Android/.test(ua) ? 'Android' : /iPhone|iPad/.test(ua) ? 'iPhone' : /Windows/.test(ua) ? 'Windows' : /Mac OS/.test(ua) ? 'Mac' : /Linux/.test(ua) ? 'Linux' : 'Browser';
  const browser = /Edg\//.test(ua) ? 'Edge' : /Chrome\//.test(ua) ? 'Chrome' : /Firefox\//.test(ua) ? 'Firefox' : /Safari\//.test(ua) ? 'Safari' : '';

  return `${os} ${browser}`.trim();
}

// --- paired ----------------------------------------------------------------------------------------------

function Home() {
  const t = useT();
  const snap = useSnapshot();
  const [tab, setTab] = useState<Tab>('inbox');

  // A tapped notification brings the open page back to the Inbox.
  useEffect(() => {
    if (snap.inboxTick > 0) setTab('inbox');
  }, [snap.inboxTick]);

  return (
    <div className="app">
      <Header />
      <Banner />
      <main className="content" id="main">
        {tab === 'inbox' && (
          <>
            <NotifyPrompt />
            <Inbox />
          </>
        )}
        {tab === 'chat' && <Chat />}
        {tab === 'files' && <Files />}
        {tab === 'more' && <More />}
      </main>
      <nav className="tabs" aria-label="Sections">
        {(['inbox', 'chat', 'files', 'more'] as const).map((id) => (
          <button key={id} className={tab === id ? 'tab active' : 'tab'} aria-current={tab === id ? 'page' : undefined} onClick={() => setTab(id)}>
            {t(id === 'inbox' ? 'tabInbox' : id === 'chat' ? 'tabChat' : id === 'files' ? 'tabFiles' : 'tabMore')}
            {id === 'inbox' && snap.cards.length + snap.pending > 0 && <span className="badge">{snap.cards.length + snap.pending}</span>}
          </button>
        ))}
      </nav>
    </div>
  );
}

function Header() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();
  const [confirming, setConfirming] = useState(false);
  const online = ['locked', 'view', 'operate'].includes(snap.link.kind);
  const label = snap.link.kind === 'operate' ? t('operate') : snap.link.kind === 'view' ? t('view') : snap.link.kind === 'locked' ? t('locked') : snap.link.kind === 'connecting' ? t('connecting') : t('computerOffline');

  return (
    <header className="header">
      <div className="status" role="status">
        <img src="/icon-192.png" alt="" width="28" height="28" />
        <span className={online ? 'dot on' : 'dot'} aria-hidden="true" />
        <span>{label}</span>
      </div>
      <button className="danger" disabled={!online} onClick={() => setConfirming(true)}>
        ■ {t('stopAll')}
      </button>
      {confirming && (
        <Dialog
          title={t('stopAll')}
          body={t('stopAllConfirm')}
          confirmLabel={t('stop')}
          cancelLabel={t('cancel')}
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            void model.kill();
          }}
        />
      )}
    </header>
  );
}

function Dialog(props: { title: string; body: string; confirmLabel: string; cancelLabel: string; onConfirm(): void; onCancel(): void }) {
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  return (
    <div className="scrim" role="presentation" onClick={props.onCancel}>
      <div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="dialog-title" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => event.key === 'Escape' && props.onCancel()}>
        <h2 id="dialog-title">{props.title}</h2>
        <p>{props.body}</p>
        <div className="row">
          <button ref={ref} onClick={props.onCancel}>{props.cancelLabel}</button>
          <button className="danger" onClick={props.onConfirm}>{props.confirmLabel}</button>
        </div>
      </div>
    </div>
  );
}

function Banner() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();

  if (snap.link.kind === 'closed') return <p className="banner error" role="alert">{snap.link.why === 'forgotten' ? '' : t('denied')}</p>;
  if (snap.link.kind === 'offline') return <p className="banner" role="status"><strong>{t('computerOffline')}.</strong> {t('computerOfflineHelp')}</p>;

  if (snap.notice === 'killed') {
    return <p className="banner ok" role="status" onClick={() => model.clearNotice()}>{t('stopAllDone')}</p>;
  }

  if (snap.notice?.startsWith('decided:')) {
    return <p className="banner" role="status" onClick={() => model.clearNotice()}>{t('decided', { who: snap.notice.slice(8) })}</p>;
  }

  if (snap.error) return <p className="banner error" role="alert">{t('failed', { why: snap.error })}</p>;

  return null;
}

/** Shown on the Inbox until notifications are on (or blocked): the permission prompt needs a tap, so this is the tap. */
function NotifyPrompt() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();
  const push = snap.push;

  if (push.kind !== 'off' && push.kind !== 'denied' && push.kind !== 'failed') return null;
  if (!['locked', 'view', 'operate'].includes(snap.link.kind)) return null;

  return (
    <section className="panel" aria-labelledby="notify-prompt">
      <h2 id="notify-prompt">{t('pushPromptTitle')}</h2>
      {push.kind === 'denied' ? (
        <p className="muted">{t('pushDeniedHow')}</p>
      ) : (
        <>
          <p className="muted">{t('pushPromptBody')}</p>
          {push.kind === 'failed' && <p role="alert" className="error">{t('pushFailed', { why: push.why })}</p>}
          <button className="primary" onClick={() => void model.push.enable()}>{t('pushEnable')}</button>
        </>
      )}
    </section>
  );
}

function Inbox() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();

  if (snap.link.kind === 'locked') {
    return (
      <section className="panel">
        {snap.pending > 0 && <p>{t('waitingLocked', { n: snap.pending })}</p>}
        <p className="muted">{t('unlockHelp')}</p>
        <button className="primary" onClick={() => void model.unlock()}>{t('unlockView')}</button>
      </section>
    );
  }

  if (snap.cards.length === 0) return <p className="muted empty">{t('inboxEmpty')}</p>;

  return (
    <div aria-live="polite">
      {snap.cards.map((card) => (
        <ApprovalCard key={card.envelope.request_id} card={card} />
      ))}
    </div>
  );
}

/** Notifications for when this page is not open. The permission prompt must come from a tap, so it is a button. */
function Notifications() {
  const t = useT();
  const model = useModel();
  const state = useSnapshot().push;
  const connected = ['locked', 'view', 'operate'].includes(useSnapshot().link.kind);

  return (
    <section className="panel" aria-labelledby="push-title">
      <h2 id="push-title">{t('pushTitle')}</h2>
      {state.kind === 'unsupported' && <p className="muted">{state.ios ? t('pushIos') : t('pushUnsupported')}</p>}
      {state.kind === 'denied' && <p className="muted">{t('pushDenied')}</p>}
      {state.kind === 'failed' && <p role="alert" className="error">{t('pushFailed', { why: state.why })}</p>}
      {(state.kind === 'off' || state.kind === 'failed') && (
        <button onClick={() => void model.push.enable()} disabled={!connected}>{t('pushEnable')}</button>
      )}
      {state.kind === 'on' && (
        <>
          <p role="status">{t('pushOn')}</p>
          <button onClick={() => void model.push.disable()}>{t('pushDisable')}</button>
        </>
      )}
      {(state.kind === 'working' || state.kind === 'checking') && <p className="muted">{t('pushWorking')}</p>}
      <p className="muted">{t('pushHelp')}</p>
    </section>
  );
}

function More() {
  const t = useT();
  const model = useModel();
  const snap = useSnapshot();

  return (
    <div>
      <section className="panel">
        <h2>{t('moreThisDevice')}</h2>
        <p>{snap.device?.name}</p>
        {snap.device?.guest && <p className="muted">{t('moreGuest')}</p>}
        {snap.device && !snap.device.guest && <p className="muted">{snap.device.recoverable ? t('moreSaved') : t('moreNotSaved')}</p>}
        <h3>{t('moreFingerprint')}</h3>
        <p className="mono big">{snap.device?.fingerprint}</p>
        <p className="muted">{t('moreFingerprintHelp')}</p>
      </section>
      <Notifications />
      <section className="panel">
        <button onClick={() => void model.lock()} disabled={snap.link.kind === 'locked'}>{t('moreLock')}</button>
      </section>
      <section className="panel">
        <button className="danger" onClick={() => void model.forget()}>{t('moreForget')}</button>
        <p className="muted">{t('moreForgetHelp')}</p>
      </section>
    </div>
  );
}
