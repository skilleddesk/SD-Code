import { ExternalLink, Globe, Play, Radio, RotateCw, Square, X } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { openOutside } from '../../lib/external';
import { previewAddress } from '../../lib/url';
import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { useRightPanelStore } from '../../store/rightPanel';
import { useAppStore } from '../../store/store';
import { joinPage, lastChange, localPort, pageChanges, previewCandidates } from './livePreview';
import { sdcpCall } from '../../lib/sdcp';
import { useSessionsStore } from '../../store/sessions';
import { toast } from '../../store/toast';
import { IconButton } from '../ui/IconButton';
import { PreviewDiff } from './PreviewDiff';
import { PreviewFile } from './PreviewFile';

/**
 * The Preview tab - spec section 7.7, and v4's live page.
 *
 * It shows, in this order: a diff (what the turn changed), the open files, or **the page the project
 * serves**. That last one used to be a placeholder with Back / Forward / Reload / Pop out buttons that
 * only toasted their own names, and an "Attach screenshot" that attached nothing. Now the address bar
 * takes a dev server's URL (`localhost:5173`, or a port on the chat's host), the frame loads it for
 * real, Reload reloads it, Open in browser opens it outside, and the device presets size the frame.
 *
 * Back and Forward are gone rather than faked: the frame is another origin, and a page on another
 * origin does not let its parent walk its history.
 */

type Device = 'mobile' | 'tablet' | 'desktop';

const DEVICE_WIDTH: Record<Device, number | null> = {
  mobile: 390,
  tablet: 768,
  desktop: null,
};

const DEVICES: readonly Device[] = ['mobile', 'tablet', 'desktop'];

/** What each chat last followed (`page edit|base`), kept across tab switches so a remount does not undo
    an address the person typed. */
const followed = new Map<string, string>();

/** The project's dev server as each chat's preview (0.15.4), kept across tab switches like `followed`. */
type DevState = { state: 'starting' | 'ready' | 'failed'; dir: string; url?: string; log?: string };
const devServers = new Map<string, DevState>();

/** A dev server on the chat's host, as an address this machine can open (`preview.forward`). */
async function forwardPort(sessionId: string, address: string): Promise<string | null> {
  const port = localPort(address);

  if (port === null) {
    return null;
  }

  try {
    const answer = await sdcpCall('preview.forward', { sessionId, port });
    const path = address.replace(/^https?:\/\/[^/]+/, '');

    return joinPage(answer.url, path === '' ? '/' : path);
  } catch (error) {
    toast(error instanceof Error ? error.message : String(error));

    return null;
  }
}

export function PreviewTab() {
  const [device, setDevice] = useState<Device>('desktop');
  const [reloads, setReloads] = useState(0);
  const diff = useFilesStore((state) => state.diff);
  const open = useFilesStore((state) => state.open);
  const { activeTab: sessionId } = useSessionsStore();
  const url = useRightPanelStore((state) => (sessionId === null ? '' : (state.previewUrls[sessionId] ?? '')));
  const [typed, setTyped] = useState<string | null>(null);
  /* Live preview (0.12.5): on by default - finds the dev server and reloads after each change. */
  const live = useRightPanelStore((state) => (sessionId === null ? true : (state.previewLive[sessionId] ?? true)));
  const turns = useAppStore((state) => state.turns);
  const projects = useAppStore((state) => state.projects);
  const hosts = useAppStore((state) => state.hosts);
  const owner = hosts.find((host) => host.sessions.some((candidate) => candidate.id === sessionId));
  const session = owner?.sessions.find((candidate) => candidate.id === sessionId);
  const project = projects.find((candidate) => candidate.id === session?.projectId);
  const onVps = owner?.type === 'vps';
  const candidates = sessionId === null ? [] : previewCandidates(turns, sessionId, project);
  const change = sessionId === null ? null : lastChange(turns, sessionId);
  const [reason, setReason] = useState<string | null>(null);
  const seenChange = useRef<string | null>(change?.key ?? null);

  /*
   * Follow the page being worked on (0.14.4). The newest edit that is a page names the path; the base
   * is the chat's dev server on this machine, or - for a chat on a VPS - the site's domain, and failing
   * that its dev server through the signed-in connection (`preview.forward`), because that chat's
   * `localhost:3000` is the VPS's, not this machine's.
   */
  const changes = sessionId === null ? [] : pageChanges(turns, sessionId, project?.root);
  const pageChange = changes.find((candidate) => candidate.page !== null) ?? null;
  const devServer = candidates.find((candidate) => localPort(candidate) !== null) ?? null;
  const siteUrl = candidates.find((candidate) => localPort(candidate) === null) ?? null;
  const [forwarded, setForwarded] = useState<Record<string, string>>({});
  const devBase = devServer === null ? null : onVps ? (forwarded[devServer] ?? null) : devServer;
  const [dev, setDevState] = useState<DevState | null>(sessionId === null ? null : (devServers.get(sessionId) ?? null));
  const setDev = (next: DevState | null): void => {
    if (sessionId !== null) {
      if (next === null) {
        devServers.delete(sessionId);
      } else {
        devServers.set(sessionId, next);
      }
    }

    setDevState(next);
  };
  /* The project's own dev server, once it runs, is what the preview shows (0.15.4): it has the page as it
     is written. Otherwise a VPS chat shows its site, a local chat the dev server it found. */
  const ownDev = dev?.state === 'ready' ? (dev.url ?? null) : null;
  const base = ownDev ?? (onVps ? (siteUrl ?? devBase) : (devBase ?? siteUrl));
  const target = base === null ? null : joinPage(base, pageChange?.page ?? '/');
  const followKey = `${pageChange?.key ?? ''}|${base ?? ''}`;

  /* A VPS chat's dev server is opened through the host, once per port. */
  useEffect(() => {
    if (!onVps || sessionId === null || devServer === null || siteUrl !== null || forwarded[devServer] !== undefined) {
      return;
    }

    void forwardPort(sessionId, devServer).then((address) => {
      if (address !== null) {
        setForwarded((known) => ({ ...known, [devServer]: address }));
      }
    });
  }, [onVps, sessionId, devServer, siteUrl, forwarded]);

  /* A new page, or a new place the site is served from, becomes what the frame shows - once each, so an
     address typed by hand stays until the agent moves to another page. */
  useEffect(() => {
    if (!live || sessionId === null || target === null || followed.get(sessionId) === followKey) {
      return;
    }

    followed.set(sessionId, followKey);

    if (target !== url) {
      useRightPanelStore.getState().setPreviewUrl(sessionId, target);
      setTyped(null);
      setReloads((count) => count + 1);
    }
  }, [live, sessionId, target, followKey, url]);

  /* Every change the agent finishes reloads the page - after a short pause, so a burst is one reload. */
  useEffect(() => {
    if (change === null || change.key === seenChange.current) {
      return;
    }

    seenChange.current = change.key;

    if (!live || url === '') {
      return;
    }

    const timer = window.setTimeout(() => {
      setReloads((count) => count + 1);
      setReason(change.target);
    }, 700);

    return () => window.clearTimeout(timer);
  }, [change, live, url]);

  /* A site on the internet is framed through the daemon's preview proxy (0.14.4): most live sites send
     `X-Frame-Options: DENY` - the user's does - and a frame drew them as a "blocked" icon. A local dev
     server is loaded as it is. The address bar and Open in browser keep the real address. */
  const [frameSrc, setFrameSrc] = useState<string>('');

  useEffect(() => {
    if (url === '' || localPort(url) !== null) {
      setFrameSrc(url);
      return;
    }

    let current = true;

    void sdcpCall('preview.open', { url }).then(
      (answer) => current && setFrameSrc(answer.url),
      () => current && setFrameSrc(url),
    );

    return () => {
      current = false;
    };
  }, [url]);

  /* Another chat's dev server is not this one's. */
  useEffect(() => {
    setDevState(sessionId === null ? null : (devServers.get(sessionId) ?? null));
  }, [sessionId]);

  /* While it starts, it is asked again every two seconds - one round trip each - for up to three minutes. */
  useEffect(() => {
    if (sessionId === null || dev?.state !== 'starting') {
      return;
    }

    let current = true;
    let tries = 0;
    const settle = (next: DevState | null): void => {
      if (next === null) {
        devServers.delete(sessionId);
      } else {
        devServers.set(sessionId, next);
      }

      setDevState(next);
    };
    const ask = (): void => {
      tries += 1;
      void sdcpCall('preview.dev', { sessionId }).then(
        (answer) => {
          if (!current) {
            return;
          }

          if (answer.state === 'starting' && tries < 90) {
            window.setTimeout(ask, 2000);

            return;
          }

          settle(
            answer.state === 'ready'
              ? { state: 'ready', dir: answer.dir, url: answer.url, log: answer.log }
              : { state: 'failed', dir: answer.dir, log: answer.log },
          );
        },
        (error: unknown) => {
          if (current) {
            settle(null);
            toast(error instanceof Error ? error.message : String(error));
          }
        },
      );
    };

    ask();

    return () => {
      current = false;
    };
  }, [sessionId, dev?.state]);

  /* A page the live site does not have yet (0.15.4): a built site answers 404 for a page whose source was
     only just written. Asked once per address the frame shows. */
  const [liveStatus, setLiveStatus] = useState<{ url: string; status: number | null } | null>(null);

  useEffect(() => {
    if (url === '' || localPort(url) !== null || ownDev !== null) {
      return;
    }

    let current = true;

    void sdcpCall('preview.status', { url }).then(
      (answer) => current && setLiveStatus({ url, status: answer.status }),
      () => undefined,
    );

    return () => {
      current = false;
    };
  }, [url, reloads, ownDev]);

  const toggleDev = (): void => {
    if (sessionId === null) {
      return;
    }

    followed.delete(sessionId);

    if (dev !== null) {
      void sdcpCall('preview.dev', { sessionId, stop: true }).catch(() => undefined);
      setDev(null);

      return;
    }

    setDev({ state: 'starting', dir: project?.root ?? '' });
  };

  if (diff !== null) {
    return <PreviewDiff />;
  }

  if (open !== null) {
    return <PreviewFile />;
  }

  const width = DEVICE_WIDTH[device];
  const field = typed ?? url;

  const go = (event: FormEvent): void => {
    event.preventDefault();

    if (sessionId === null) {
      return;
    }

    const address = previewAddress(field);

    if (address === null && field.trim() !== '') {
      toast(strings.rightPanel.preview.badUrl);

      return;
    }

    useRightPanelStore.getState().setPreviewUrl(sessionId, address ?? '');
    setTyped(null);
    setReloads((count) => count + 1);
  };

  return (
    <div className="preview flex min-h-0 flex-1 flex-col">
      <form className="preview-toolbar flex items-center gap-[4px] border-b border-border-subtle px-[10px] py-[8px]" onSubmit={go}>
        <IconButton
          icon={RotateCw}
          label={strings.rightPanel.preview.reload}
          iconSize={14}
          disabled={url === ''}
          onClick={() => setReloads((count) => count + 1)}
        />

        <label className="sr-only" htmlFor="previewUrl">
          {strings.rightPanel.preview.address}
        </label>
        <div className="preview-url mx-[4px] flex min-w-0 flex-1 items-center gap-[6px] rounded-md border border-border-subtle bg-bg-input px-[8px] focus-within:border-border-focus">
          <Globe size={11} className="shrink-0 text-text-muted" aria-hidden="true" />
          <input
            id="previewUrl"
            className="h-[26px] min-w-0 flex-1 bg-transparent font-mono text-[11px] text-text-primary placeholder:text-text-muted"
            placeholder={strings.rightPanel.preview.placeholder}
            value={field}
            spellCheck={false}
            autoComplete="off"
            disabled={sessionId === null}
            onChange={(event) => setTyped(event.target.value)}
          />
          {url === '' ? null : (
            <button
              type="button"
              className="grid h-[18px] w-[18px] place-items-center rounded-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
              aria-label={strings.rightPanel.preview.clear}
              onClick={() => {
                if (sessionId !== null) {
                  useRightPanelStore.getState().setPreviewUrl(sessionId, '');
                  setTyped(null);
                }
              }}
            >
              <X size={10} aria-hidden="true" />
            </button>
          )}
        </div>

        <IconButton
          icon={ExternalLink}
          label={strings.rightPanel.preview.popOut}
          iconSize={14}
          disabled={url === ''}
          onClick={() => void openOutside(url)}
        />

        <button
          type="button"
          aria-pressed={dev !== null}
          title={dev === null ? strings.rightPanel.preview.devStart : strings.rightPanel.preview.devStop}
          disabled={sessionId === null || project === undefined}
          className={
            'preview-dev ml-[2px] flex h-[26px] shrink-0 items-center gap-[5px] rounded-md border px-[8px] text-[10.5px] font-semibold transition-colors duration-fast ' +
            (dev !== null
              ? 'border-accent/40 bg-accent-subtle text-accent'
              : 'border-border-subtle bg-bg-raised text-text-muted hover:text-text-secondary')
          }
          onClick={toggleDev}
        >
          {dev === null ? <Play size={11} aria-hidden="true" /> : <Square size={10} aria-hidden="true" />}
          {strings.rightPanel.preview.dev}
        </button>

        <button
          type="button"
          role="switch"
          aria-checked={live}
          title={live ? strings.rightPanel.preview.liveOn : strings.rightPanel.preview.liveOff}
          disabled={sessionId === null}
          className={
            'preview-live ml-[2px] flex h-[26px] shrink-0 items-center gap-[5px] rounded-md border px-[8px] text-[10.5px] font-semibold transition-colors duration-fast ' +
            (live
              ? 'border-state-success/40 bg-green-subtle text-state-success'
              : 'border-border-subtle bg-bg-raised text-text-muted hover:text-text-secondary')
          }
          onClick={() => {
            if (sessionId !== null) {
              useRightPanelStore.getState().setPreviewLive(sessionId, !live);
            }
          }}
        >
          <Radio size={11} aria-hidden="true" className={live ? 'animate-pulse motion-reduce:animate-none' : undefined} />
          {strings.rightPanel.preview.live}
        </button>
      </form>

      {candidates.filter((candidate) => candidate !== url).length > 0 ? (
        <div className="preview-found flex flex-wrap items-center gap-[4px] px-[10px] pt-[8px]">
          <span className="text-[10px] font-semibold uppercase tracking-[.08em] text-text-muted">{strings.rightPanel.preview.found}</span>
          {candidates
            .filter((candidate) => candidate !== url)
            .map((candidate) => (
              <button
                key={candidate}
                type="button"
                className="max-w-full truncate rounded-sm border border-border-subtle bg-bg-raised px-[7px] py-[2px] font-mono text-[10px] text-accent hover:border-border-default"
                title={candidate}
                onClick={() => {
                  if (sessionId === null) {
                    return;
                  }

                  const show = (address: string): void => {
                    useRightPanelStore.getState().setPreviewUrl(sessionId, joinPage(address, pageChange?.page ?? '/'));
                    setTyped(null);
                    setReloads((count) => count + 1);
                  };

                  /* On a VPS, `localhost` is the VPS's: it is opened through the host. */
                  if (onVps && localPort(candidate) !== null) {
                    void forwardPort(sessionId, candidate).then((address) => {
                      if (address !== null) {
                        setForwarded((known) => ({ ...known, [candidate]: address }));
                        show(address);
                      }
                    });

                    return;
                  }

                  show(candidate);
                }}
              >
                {onVps && localPort(candidate) !== null
                  ? strings.rightPanel.preview.onHost(owner?.name ?? '', localPort(candidate) ?? 0)
                  : candidate.replace(/^https?:\/\//, '').replace(/\/$/, '')}
              </button>
            ))}
        </div>
      ) : null}

      {dev !== null ? (
        <div
          className={
            'preview-dev-note mx-[10px] mt-[8px] rounded-md border px-[8px] py-[5px] text-[11px] leading-[1.45] ' +
            (dev.state === 'failed' ? 'border-state-error/40 text-state-error' : 'border-border-subtle text-text-secondary')
          }
          role="status"
        >
          {dev.state === 'starting'
            ? strings.rightPanel.preview.devStarting(dev.dir)
            : dev.state === 'failed'
              ? strings.rightPanel.preview.devFailed(dev.dir)
              : strings.rightPanel.preview.devOn(dev.dir)}
          {dev.state === 'failed' && dev.log !== undefined && dev.log.trim() !== '' ? (
            <pre className="mt-[4px] max-h-[120px] overflow-auto whitespace-pre-wrap font-mono text-[10px] text-text-muted">{dev.log}</pre>
          ) : null}
        </div>
      ) : liveStatus !== null && liveStatus.url === url && liveStatus.status !== null && liveStatus.status >= 400 ? (
        <div
          className="preview-not-live mx-[10px] mt-[8px] flex flex-wrap items-center gap-[6px] rounded-md border border-state-warning/40 px-[8px] py-[5px] text-[11px] leading-[1.45] text-text-secondary"
          role="status"
        >
          <span className="min-w-0 flex-1">
            {strings.rightPanel.preview.notLive(url.replace(/^https?:\/\/[^/]+/, '') || '/', liveStatus.status)}
          </span>
          <button
            type="button"
            className="shrink-0 rounded-sm border border-border-default bg-bg-raised px-[7px] py-[2px] text-[10.5px] font-semibold text-accent hover:border-border-focus"
            onClick={toggleDev}
          >
            {strings.rightPanel.preview.notLiveAction}
          </button>
        </div>
      ) : null}

      <div className="device-presets flex gap-[4px] px-[10px] pt-[8px]">
        {DEVICES.map((candidate) => (
          <button
            key={candidate}
            type="button"
            className={
              'device-preset shrink-0 whitespace-nowrap rounded-sm border px-[8px] py-[3px] font-mono text-[10px] transition-all duration-fast ' +
              (candidate === device
                ? 'active border-border-focus bg-accent-subtle text-accent'
                : 'border-border-subtle bg-bg-raised text-text-secondary hover:border-border-default hover:text-text-primary')
            }
            aria-pressed={candidate === device}
            onClick={() => setDevice(candidate)}
          >
            {strings.rightPanel.preview.devices[candidate]}
          </button>
        ))}
        {live && pageChange !== null && pageChange.page !== null && url === target ? (
          <span
            className="preview-following ml-auto min-w-0 truncate self-center font-mono text-[10px] text-text-muted"
            title={pageChange.file}
            data-preview-page={pageChange.page}
          >
            {strings.rightPanel.preview.following(pageChange.page, pageChange.file.replace(/^.*[\\/]/, ''))}
          </span>
        ) : live && reason !== null && url !== '' ? (
          <span className="ml-auto min-w-0 truncate self-center font-mono text-[10px] text-text-muted" title={reason}>
            {strings.rightPanel.preview.reloadedAfter(reason)}
          </span>
        ) : null}
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-[10px] py-[10px]">
        {url === '' ? (
          <div
            className="preview-frame mx-auto grid min-h-[220px] w-full place-items-center rounded-md border border-dashed border-border-default bg-bg-base"
            id="previewEmpty"
          >
            <div className="p-[20px] text-center">
              <div className="mb-[6px] text-[13px] font-medium text-text-secondary">{strings.rightPanel.preview.emptyTitle}</div>
              <div className="mx-auto max-w-[300px] text-[11.5px] leading-[1.55] text-text-muted">
                {live ? strings.rightPanel.preview.emptyLive : strings.rightPanel.preview.emptyBody}
              </div>
            </div>
          </div>
        ) : (
          <div
            className="preview-frame mx-auto h-full min-h-[320px] w-full overflow-hidden rounded-md border border-border-default bg-white"
            style={width === null ? undefined : { maxWidth: width }}
          >
            <iframe
              key={`${frameSrc}#${reloads}`}
              src={frameSrc}
              title={strings.rightPanel.preview.frameTitle(url)}
              className="h-full min-h-[320px] w-full border-0"
              sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals"
            />
          </div>
        )}
      </div>
    </div>
  );
}
