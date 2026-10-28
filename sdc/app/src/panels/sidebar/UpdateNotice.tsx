import { ArrowUpCircle, LoaderCircle, X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { checkAppUpdate, inShell, installAppUpdate } from '../../lib/appUpdate';
import { openOutside } from '../../lib/external';
import { storedSettings, storeSetting } from '../../lib/settings';
import { sdcpCall } from '../../lib/sdcp';
import { useOverlayStore } from '../../store/overlays';
import { strings } from '../../strings';

/**
 * The update notice at the foot of the sidebar (1.0.3).
 *
 * The report: "when a new version is on GitHub, every SDC must say so - at the bottom of the sidebar, with the version,
 * a cross to close it, and a way to skip it; it comes back every time SDC opens until it is updated". So:
 *
 *   - SDC asks on start and every six hours: the desktop shell's signed updater first (it can install), else the
 *     daemon's GitHub check (it can only link to the release);
 *   - **Update now** downloads, checks the signature and installs in place, with its progress in the card - or opens
 *     the release page where there is no shell to install with;
 *   - **Not now** and the cross hide it until SDC is opened again;
 *   - **Skip this version** hides that version for good - the next one is offered as usual.
 *
 * Quiet on purpose: a check that fails (offline, rate-limited) shows nothing, and Settings -> Updates still says why.
 */

const EVERY_MS = 6 * 60 * 60 * 1000;
const SKIP_KEY = 'update-skip';

/** Dismissed with "Not now" in this run of SDC: shown again at the next start. */
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

  if (offer === null || hidden || dismissedThisRun === offer.version || storedSettings()[SKIP_KEY] === offer.version) {
    return null;
  }

  const notNow = (): void => {
    dismissedThisRun = offer.version;
    setHidden(true);
  };

  const skip = (): void => {
    storeSetting(SKIP_KEY, offer.version);
    setHidden(true);
  };

  const update = (): void => {
    if (!offer.installable) {
      if (offer.url !== null) {
        void openOutside(offer.url);
      } else {
        useOverlayStore.getState().openSettings('updates');
      }

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

  return <UpdateCard offer={offer} progress={progress} failed={failed} onUpdate={update} onNotNow={notNow} onSkip={skip} />;
}

export interface UpdateCardProps {
  offer: Offer;
  progress: number | 'unknown' | 'installing' | null;
  failed: boolean;
  onUpdate: () => void;
  onNotNow: () => void;
  onSkip: () => void;
}

/** The notice itself, without the checking: what the sidebar draws for an offer. */
export function UpdateCard({ offer, progress, failed, onUpdate: update, onNotNow: notNow, onSkip: skip }: UpdateCardProps) {
  const u = strings.sidebar.update;
  const busy = progress !== null;

  return (
    <div className="update-notice shrink-0 px-[10px] pb-[10px] pt-[6px]" data-update-notice={offer.version}>
      <div
        className="relative overflow-hidden rounded-lg border border-accent/30 bg-[image:var(--grad-brand-soft)] p-[10px] shadow-[inset_0_1px_0_var(--edge-light)] animate-drop-up"
        role="status"
        aria-live="polite"
      >
        {busy ? null : (
          <button
            type="button"
            className="absolute right-[6px] top-[6px] grid h-[22px] w-[22px] place-items-center rounded-full text-text-muted transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary"
            title={u.notNow}
            aria-label={u.close}
            onClick={notNow}
          >
            <X size={12} aria-hidden="true" />
          </button>
        )}

        <div className="flex items-start gap-[9px] pr-[18px]">
          <span className="brand-fill mt-[1px] grid h-[26px] w-[26px] shrink-0 place-items-center rounded-full" aria-hidden="true">
            {busy ? <LoaderCircle size={14} className="animate-spin motion-reduce:animate-none" /> : <ArrowUpCircle size={15} />}
          </span>
          <span className="min-w-0">
            <span className="block text-[12.5px] font-semibold leading-[1.35] text-text-primary">{u.title(offer.version)}</span>
            <span className="block text-[11px] leading-[1.45] text-text-secondary">
              {progress === 'installing'
                ? u.installing
                : busy
                  ? u.downloading(typeof progress === 'number' ? progress : null)
                  : failed
                    ? u.failed
                    : u.youHave(offer.current)}
            </span>
          </span>
        </div>

        {busy ? (
          <div className="mt-[8px] h-[4px] w-full overflow-hidden rounded-full bg-bg-input" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={typeof progress === 'number' ? progress : undefined}>
            <div className="h-full bg-[image:var(--grad-brand)] transition-[width] duration-base" style={{ width: `${typeof progress === 'number' ? progress : progress === 'installing' ? 100 : 12}%` }} />
          </div>
        ) : (
          <div className="mt-[9px] flex items-center gap-[6px]">
            <button
              type="button"
              className="brand-fill inline-flex h-[26px] items-center rounded-md px-[10px] text-[11px] font-semibold"
              data-update-now
              onClick={update}
            >
              {offer.installable ? u.updateNow : u.download}
            </button>
            <button
              type="button"
              className="inline-flex h-[26px] items-center rounded-md px-[8px] text-[11px] font-medium text-text-secondary transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary"
              onClick={notNow}
            >
              {u.notNow}
            </button>
            <button
              type="button"
              className="ml-auto inline-flex h-[26px] items-center rounded-md px-[6px] text-[10.5px] text-text-muted transition-colors duration-fast hover:text-text-primary hover:underline"
              title={u.skipHelp}
              onClick={skip}
            >
              {u.skip}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
