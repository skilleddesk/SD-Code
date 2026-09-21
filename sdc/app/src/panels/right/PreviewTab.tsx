import { Camera, ChevronLeft, ChevronRight, ExternalLink, RotateCw } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { toast } from '../../store/toast';
import { BTN, BTN_BLOCK, BTN_SECONDARY } from '../ui/button';
import { IconButton } from '../ui/IconButton';
import { PreviewDiff } from './PreviewDiff';
import { PreviewFile } from './PreviewFile';

/**
 * The Preview tab - spec section 7.7.
 *
 * A browser toolbar (back, forward, reload, a mono URL, pop out), three device presets, the framed
 * page, and a full-width `Attach screenshot` footer.
 *
 * The frame is a 16:10 placeholder rather than a real web view: it is a two-stop gradient with two
 * radial glows over it, the page's name in large type and its path in mono underneath. The gradient
 * is a token (`--grad-preview`) because it is the one surface in the app that has a light-theme
 * value of its own.
 *
 * Tablet 768 is the default preset, and the preset actually resizes the frame - the spec's
 * `Mobile 390 / Tablet 768 / Desktop` would be decoration otherwise. In a 400px panel "Tablet" is
 * simply full width, which is what the prototype shows; drag the panel open and it becomes a real
 * constraint.
 */

type Device = 'mobile' | 'tablet' | 'desktop';

const DEVICE_WIDTH: Record<Device, number | null> = {
  mobile: 390,
  tablet: 768,
  desktop: null,
};

const DEVICES: readonly Device[] = ['mobile', 'tablet', 'desktop'];

export function PreviewTab() {
  const [device, setDevice] = useState<Device>('tablet');
  const diff = useFilesStore((state) => state.diff);
  const open = useFilesStore((state) => state.open);

  /*
   * The tab shows, in this order: a diff (0.7.9, what the turn changed), a file (0.7.7, what is in the
   * folder), or the web preview below - which SDCP 0.1 cannot fill until a URL is attached
   * (`console.attach`), so it says so rather than drawing a page nobody started.
   */
  if (diff !== null) {
    return <PreviewDiff />;
  }

  if (open !== null) {
    return <PreviewFile />;
  }

  const width = DEVICE_WIDTH[device];

  return (
    <>
      <div className="preview-toolbar flex items-center gap-[4px] border-b border-border-subtle px-[10px] py-[8px]">
        <IconButton
          icon={ChevronLeft}
          label={strings.rightPanel.preview.back}
          iconSize={14}
          onClick={() => toast(strings.rightPanel.preview.back)}
        />
        <IconButton
          icon={ChevronRight}
          label={strings.rightPanel.preview.forward}
          iconSize={14}
          onClick={() => toast(strings.rightPanel.preview.forward)}
        />
        <IconButton
          icon={RotateCw}
          label={strings.rightPanel.preview.reload}
          iconSize={14}
          onClick={() => toast(strings.rightPanel.preview.reload)}
        />

        <div className="preview-url mx-[6px] min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap rounded-md border border-border-subtle bg-bg-input px-[10px] py-[5px] font-mono text-[10.5px] text-text-muted">
          {/*
            No URL, and it says so. SDCP 0.1 has no event that carries an attached page (`console.attach`
            hands the daemon a URL and the daemon answers with the checks it ran), so the box shows the
            `noUrl` sentence rather than a hardcoded `localhost:5173/login` - which read as a working
            preview of a dev server this window had never started. A file opened from the tree replaces
            this whole tab; see `PreviewFile`.
          */}
          {strings.rightPanel.preview.noUrl}
        </div>

        <IconButton
          icon={ExternalLink}
          label={strings.rightPanel.preview.popOut}
          iconSize={14}
          onClick={() => toast(strings.rightPanel.preview.popOut)}
        />
      </div>

      <div className="device-presets flex gap-[4px] px-[10px] pt-[8px]">
        {DEVICES.map((candidate) => (
          <button
            key={candidate}
            type="button"
            className={
              'device-preset rounded-sm border px-[8px] py-[3px] font-mono text-[10px] transition-all duration-fast ' +
              (candidate === device
                ? 'active border-[rgba(91,156,255,.3)] bg-accent-subtle text-accent'
                : 'border-border-subtle bg-bg-raised text-text-secondary hover:border-border-default hover:text-text-primary')
            }
            onClick={() => {
              setDevice(candidate);
              toast(strings.rightPanel.preview.deviceToast(DEVICE_WIDTH[candidate]));
            }}
          >
            {strings.rightPanel.preview.devices[candidate]}
          </button>
        ))}
      </div>

      {/*
        The frame, and what it is allowed to draw.
        It used to be a `Login / src/routes/login.tsx` mock - a gradient with two glows and a page
        name - which read as a working preview of a project the window had never opened. Until a URL
        is actually attached (`console.attach`, spec section 15.4), an empty panel is the truth.
      */}
      <div
        className="preview-frame relative mx-auto my-[12px] grid aspect-[16/10] w-[calc(100%-24px)] place-items-center overflow-hidden rounded-md border border-border-subtle bg-bg-base"
        style={width === null ? undefined : { maxWidth: width }}
        id="previewEmpty"
      >
        <div className="p-[20px] text-center">
          <div className="mb-[6px] text-[13px] font-medium text-text-secondary">
            {strings.rightPanel.preview.emptyTitle}
          </div>
          <div className="mx-auto max-w-[280px] text-[11.5px] text-text-muted">
            {strings.rightPanel.preview.emptyBody}
          </div>
        </div>
      </div>

      <div className="px-[12px] pb-[12px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_BLOCK}
          onClick={() => toast(strings.rightPanel.preview.attached)}
        >
          <Camera size={12} aria-hidden="true" />
          {strings.rightPanel.preview.attach}
        </button>
      </div>
    </>
  );
}
