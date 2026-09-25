import { ExternalLink, Globe, RotateCw, X } from 'lucide-react';
import { useState, type FormEvent } from 'react';

import { previewAddress } from '../../lib/url';
import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { useRightPanelStore } from '../../store/rightPanel';
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

/** Opens a URL in the person's own browser - through Tauri's shell in the app, a new tab in dev. */
async function openOutside(url: string): Promise<void> {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');

    await open(url);
  } catch {
    window.open(url, '_blank', 'noopener');
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
      </form>

      <div className="device-presets flex gap-[4px] px-[10px] pt-[8px]">
        {DEVICES.map((candidate) => (
          <button
            key={candidate}
            type="button"
            className={
              'device-preset rounded-sm border px-[8px] py-[3px] font-mono text-[10px] transition-all duration-fast ' +
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
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-[10px] py-[10px]">
        {url === '' ? (
          <div
            className="preview-frame mx-auto grid min-h-[220px] w-full place-items-center rounded-md border border-dashed border-border-default bg-bg-base"
            id="previewEmpty"
          >
            <div className="p-[20px] text-center">
              <div className="mb-[6px] text-[13px] font-medium text-text-secondary">{strings.rightPanel.preview.emptyTitle}</div>
              <div className="mx-auto max-w-[300px] text-[11.5px] leading-[1.55] text-text-muted">{strings.rightPanel.preview.emptyBody}</div>
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
