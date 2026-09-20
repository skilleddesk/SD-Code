import { Camera, ChevronLeft, ChevronRight, ExternalLink, RotateCw } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { BTN, BTN_BLOCK, BTN_SECONDARY } from '../ui/button';
import { IconButton } from '../ui/IconButton';

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

        <div className="preview-url mx-[6px] min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap rounded-md border border-border-subtle bg-bg-input px-[10px] py-[5px] font-mono text-[10.5px] text-text-secondary">
          {strings.rightPanel.preview.url}
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

      <div
        className="preview-frame relative mx-auto my-[12px] grid aspect-[16/10] w-[calc(100%-24px)] place-items-center overflow-hidden rounded-md border border-border-subtle [background-image:var(--grad-preview)] before:absolute before:inset-0 before:content-[''] before:[background-image:var(--grad-preview-glow)]"
        style={width === null ? undefined : { maxWidth: width }}
      >
        <div className="placeholder relative p-[20px] text-center">
          <div className="big mb-[6px] text-[22px] font-bold text-text-secondary">
            {strings.rightPanel.preview.page.name}
          </div>
          <div className="sub font-mono text-[11px] text-text-muted">
            {strings.rightPanel.preview.page.path}
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
