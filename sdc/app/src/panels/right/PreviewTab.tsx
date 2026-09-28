import { ExternalLink, Globe, Radio, RotateCw, X } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { openOutside } from '../../lib/external';
import { previewAddress } from '../../lib/url';
import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { useRightPanelStore } from '../../store/rightPanel';
import { useAppStore } from '../../store/store';
import { lastChange, previewCandidates } from './livePreview';
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
  const session = hosts.flatMap((host) => host.sessions).find((candidate) => candidate.id === sessionId);
  const project = projects.find((candidate) => candidate.id === session?.projectId);
  const candidates = sessionId === null ? [] : previewCandidates(turns, sessionId, project);
  const change = sessionId === null ? null : lastChange(turns, sessionId);
  const [reason, setReason] = useState<string | null>(null);
  const seenChange = useRef<string | null>(change?.key ?? null);
  const newest = candidates[0] ?? '';
  const seenCandidate = useRef<string>(newest);

  /* A dev server the chat just started becomes the page, when nothing is previewed yet. */
  useEffect(() => {
    if (!live || sessionId === null || newest === '' || newest === seenCandidate.current) {
      return;
    }

    seenCandidate.current = newest;

    if (url === '' && !newest.startsWith('https://')) {
      useRightPanelStore.getState().setPreviewUrl(sessionId, newest);
    }
  }, [live, sessionId, newest, url]);

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
                  if (sessionId !== null) {
                    useRightPanelStore.getState().setPreviewUrl(sessionId, candidate);
                    setTyped(null);
                    setReloads((count) => count + 1);
                  }
                }}
              >
                {candidate.replace(/^https?:\/\//, '').replace(/\/$/, '')}
              </button>
            ))}
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
        {live && reason !== null && url !== '' ? (
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
              key={`${url}#${reloads}`}
              src={url}
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
