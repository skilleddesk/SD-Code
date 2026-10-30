import { ArrowRight, Download, LoaderCircle, Sparkles, X } from 'lucide-react';
import type { CSSProperties } from 'react';
import { useEffect, useRef, useState } from 'react';

import { checkAppUpdate, inShell, installAppUpdate } from '../lib/appUpdate';
import { openOutside } from '../lib/external';
import { storedSettings } from '../lib/settings';
import { sdcpCall } from '../lib/sdcp';
import { useOverlayStore } from '../store/overlays';
import { strings } from '../strings';

/**
 * The update notice (1.0.3; a top-right card since 1.0.9).
 *
 * The report (1.0.9): "it comes in on the right, goes away by itself after a while, has a cross to close it, and comes
 * back every time SDC is opened - otherwise not; and it fits any window size". So:
 *
 *   - SDC asks on start and every six hours: the desktop shell's signed updater first (it can install), else the
 *     daemon's GitHub check (it can only link to the release);
 *   - the card slides in at the top right, outside the panels, and is as wide as the window allows;
 *   - left alone it leaves after {@link SHOW_MS} - the bar along its foot drains meanwhile, and stops while the pointer
 *     or the keyboard is on the card; while an update downloads it stays;
 *   - **Update now** downloads, checks the signature and installs in place, with its progress in the card - or opens
 *     the release page where there is no shell to install with;
 *   - **Later**, the cross and the timer hide it until SDC is opened again. There is no "skip": the next start asks
 *     again, until SDC is updated.
 *
 * Quiet on purpose: a check that fails (offline, rate-limited) shows nothing, and Settings -> Updates still says why.
 */

const EVERY_MS = 6 * 60 * 60 * 1000;
/** How long the card stays when nobody touches it. */
const SHOW_MS = 15_000;

/** Closed (by hand or by the timer) in this run of SDC: shown again at the next start. */
let dismissedThisRun: string | null = null;

export interface Offer {
  version: string;
  current: string;
  /** The shell can install it here. */
  installable: boolean;
  url: string | null;
}

const bare = (version: string): string => version.trim().replace(/^v/i, '');

async function findOffer(): Promise<Offer | null> {
  if (inShell()) {
    try {
      const app = await checkAppUpdate();

      if (app.available && app.version !== undefined) {
        return { version: bare(app.version), current: bare(app.current), installable: true, url: null };
      }

      return null;
    } catch {
      /* No signed manifest for this release yet: the daemon's check still says what exists. */
    }
  }

  try {
    const channel = storedSettings()['update-channel'] === 'beta' ? 'beta' : 'stable';
    const info = await sdcpCall('update.check', { channel });

    if (info?.updateAvailable === true && info.latest !== null) {
      return { version: bare(info.latest.tag), current: bare(info.current), installable: false, url: info.latest.url };
    }
  } catch {
    /* Offline or rate-limited: nothing to say, and nothing to nag about. */
  }

  return null;
}

export function UpdateNotice() {
  const [offer, setOffer] = useState<Offer | null>(null);
  const [hidden, setHidden] = useState(false);
  /** `null` idle; a number (or `'unknown'`) while downloading; `'installing'` once the files are in. */
  const [progress, setProgress] = useState<number | 'unknown' | 'installing' | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    const look = (): void => {
      void findOffer().then((found) => {
        if (alive) {
          setOffer(found);
        }
      });
    };
    /* A moment after start, so the first paint and the daemon's own start come first. */
    const first = window.setTimeout(look, 4000);
    const timer = window.setInterval(look, EVERY_MS);

    return () => {
      alive = false;
      window.clearTimeout(first);
      window.clearInterval(timer);
    };
  }, []);

  if (offer === null || hidden || dismissedThisRun === offer.version) {
    return null;
  }

  const close = (): void => {
    dismissedThisRun = offer.version;
    setHidden(true);
  };

  const update = (): void => {
    if (!offer.installable) {
      if (offer.url !== null) {
        void openOutside(offer.url);
      } else {
        useOverlayStore.getState().openSettings('updates');
      }

      close();

      return;
    }

    setFailed(false);
    setProgress('unknown');
    void installAppUpdate((percent) => setProgress(percent === null ? 'unknown' : percent >= 100 ? 'installing' : percent))
      .then(() => setProgress('installing'))
      .catch(() => {
        setProgress(null);
        setFailed(true);
      });
  };

  return <UpdateCard key={offer.version} offer={offer} progress={progress} failed={failed} onUpdate={update} onClose={close} />;
}

export interface UpdateCardProps {
  offer: Offer;
  progress: number | 'unknown' | 'installing' | null;
  failed: boolean;
  onUpdate: () => void;
  /** Later, the cross, or the timer: hidden until SDC opens again. */
  onClose: () => void;
}

/** The card itself, without the checking: what the window draws for an offer. */
export function UpdateCard({ offer, progress, failed, onUpdate, onClose }: UpdateCardProps) {
  const u = strings.sidebar.update;
  const busy = progress !== null;
  /* The timer only runs while nobody is reading or using the card, and never during a download or after a failure. */
  const [held, setHeld] = useState(false);
  const counting = !busy && !failed && !held;
  const left = useRef(SHOW_MS);
  /* What the bar draws: the time still left when the timer last stopped (a ref is not read while drawing). */
  const [remaining, setRemaining] = useState(SHOW_MS);

  /* The latest close, so a parent's new function does not restart the timer. */
  const closeRef = useRef(onClose);

  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!counting) {
      return;
    }

    const started = performance.now();
    const timer = window.setTimeout(() => closeRef.current(), left.current);

    return () => {
      window.clearTimeout(timer);
      left.current = Math.max(1500, left.current - (performance.now() - started));
      setRemaining(left.current);
    };
  }, [counting]);

  const percent = typeof progress === 'number' ? progress : progress === 'installing' ? 100 : null;

  return (
    <div
      className="update-notice pointer-events-none fixed right-[12px] top-[56px] z-[1900] flex w-[min(360px,calc(100vw-24px))] justify-end"
      data-update-notice={offer.version}
    >
      <section
        className="pointer-events-auto relative w-full overflow-hidden rounded-xl border border-accent/25 bg-bg-overlay shadow-[0_18px_48px_-12px_rgba(0,0,0,.55),inset_0_1px_0_var(--edge-light)] animate-slide-in-right motion-reduce:animate-none"
        role="status"
        aria-live="polite"
        aria-label={u.title(offer.version)}
        onMouseEnter={() => setHeld(true)}
        onMouseLeave={() => setHeld(false)}
        onFocus={() => setHeld(true)}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
            setHeld(false);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && !busy) {
            onClose();
          }
        }}
      >
        {/* The brand's colour along the top edge: an update, not an error. */}
        <div className="h-[3px] w-full bg-[image:var(--grad-brand)]" aria-hidden="true" />

        <div className="flex items-start gap-[12px] px-[14px] pb-[12px] pt-[12px]">
          <span className="brand-fill grid h-[36px] w-[36px] shrink-0 place-items-center rounded-lg" aria-hidden="true">
            {busy ? <LoaderCircle size={18} className="animate-spin motion-reduce:animate-none" /> : <Sparkles size={18} />}
          </span>

          <div className="min-w-0 flex-1">
            <div className="flex items-start gap-[8px]">
              <h2 className="m-0 min-w-0 flex-1 text-[13.5px] font-semibold leading-[1.35] text-text-primary">{u.title(offer.version)}</h2>
              {busy ? null : (
                <button
                  type="button"
                  className="-mr-[4px] -mt-[2px] grid h-[24px] w-[24px] shrink-0 place-items-center rounded-md text-text-muted transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
                  title={u.close}
                  aria-label={u.close}
                  onClick={onClose}
                >
                  <X size={14} aria-hidden="true" />
                </button>
              )}
            </div>

            {/* What changes: the version here, and the one on offer. */}
            <div className="mt-[6px] flex flex-wrap items-center gap-[6px] font-mono text-[11px]" aria-label={u.youHave(offer.current)}>
              <span className="rounded-md border border-border-subtle bg-bg-raised px-[6px] py-[1px] text-text-muted">v{offer.current}</span>
              <ArrowRight size={12} className="text-text-muted" aria-hidden="true" />
              <span className="rounded-md border border-accent/40 bg-accent/10 px-[6px] py-[1px] font-semibold text-text-primary">v{offer.version}</span>
            </div>

            <p className={'m-0 mt-[8px] text-[12px] leading-[1.5] ' + (failed ? 'text-state-error' : 'text-text-secondary')}>
              {progress === 'installing' ? u.installing : busy ? u.downloading(typeof progress === 'number' ? progress : null) : failed ? u.failed : u.body}
            </p>

            {busy ? (
              <div
                className="mt-[10px] h-[6px] w-full overflow-hidden rounded-full bg-bg-input"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={percent ?? undefined}
              >
                <div className="h-full rounded-full bg-[image:var(--grad-brand)] transition-[width] duration-base" style={{ width: `${percent ?? 12}%` }} />
              </div>
            ) : (
              <div className="mt-[12px] flex flex-wrap items-center gap-[8px]">
                <button
                  type="button"
                  className="brand-fill inline-flex h-[30px] min-w-0 flex-1 items-center justify-center gap-[6px] rounded-lg px-[12px] text-[12px] font-semibold focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                  data-update-now
                  onClick={onUpdate}
                >
                  <Download size={13} aria-hidden="true" />
                  {offer.installable ? u.updateNow : u.download}
                </button>
                <button
                  type="button"
                  className="inline-flex h-[30px] items-center justify-center rounded-lg border border-border-default px-[12px] text-[12px] font-medium text-text-secondary transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
                  onClick={onClose}
                >
                  {u.notNow}
                </button>
              </div>
            )}
          </div>
        </div>

        {/* How long it stays: drains while nobody is on the card, and stands still while someone is. */}
        {busy || failed ? null : (
          <div className="h-[2px] w-full bg-bg-input" aria-hidden="true">
            <div
              key={remaining}
              className="h-full origin-left bg-accent/70 animate-drain motion-reduce:animate-none"
              style={
                {
                  '--drain-from': remaining / SHOW_MS,
                  animationDuration: `${remaining}ms`,
                  animationPlayState: counting ? 'running' : 'paused',
                } as CSSProperties
              }
            />
          </div>
        )}
      </section>
    </div>
  );
}
