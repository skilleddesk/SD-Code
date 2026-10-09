import { Search } from 'lucide-react';

import { KillSwitch } from '../../kernel/KillSwitch';

import { framelessWindow } from '../../lib/frameless';
import { strings } from '../../strings';
import { useOverlayStore } from '../../store/overlays';
import { HostPill } from './HostPill';
import { ModeSwitch } from './ModeSwitch';
import { WindowControls } from './WindowControls';
/**
 * The topbar region - spec section 7.1, eleven elements left to right.
 *
 *   1  .brand-mark   24x24 gradient "S"          click -> About, in the Settings modal
 *   2  .brand-text   "SDC"                       hidden at 640px (src/layout/Shell.css)
 *   3  #activeHostBtn host pill                  -> host switcher popover
 *   4  .spacer       flex:1
 *   5  .mode-switch  Simple / Pro / Auto         hidden at 900px
 *   6  #openPalette  Search or jump to…  ⌘K      collapses to its icon at 1100px
 *   7  #openProviders plug, has-dot              -> Provider Hub
 *   8  #themeToggle  moon / sun                  dark <-> light
 *   9  #toggleSidebar panel-left                 Ctrl+B
 *  10  #toggleRight  panel-right                 Ctrl+J
 *  11  #openSettings settings gear               -> Settings modal
 *
 * The region element itself (`.topbar`) is the shell's: its height, hairline and padding come from
 * src/layout/Shell.css, and this component only fills it.
 *
 * Two of the eleven are stubs by design at this point in the build: the palette (spec section 9.2)
 * and Settings (9.11) are later steps. Both still dispatch - the overlay store flips its flag, so
 * the surface has a place to mount, and a toast answers the click immediately. The plug button is
 * fully working: its dot is driven by the provider store, not by a flag.
 *
 * The theme toggle is the one button with a side effect outside React: `applyTheme` writes
 * `<html data-theme>`, which is what the token cascade keys off, and the store records it so the
 * button's own icon and the Appearance tab agree.
 */
export function Topbar() {
  const openPalette = useOverlayStore((state) => state.openPalette);

  /* 0.19: the navigation moved to the rail (panels/rail/NavRail.tsx); the bar keeps what describes the
     present - the machine, the mode, the command search and Stop all. 0.21: on every platform it is also the
     window's caption - empty space drags the window, a double-click maximises, and the window buttons
     close the row. */
  return (
    <header className={'topbar' + (framelessWindow() ? ' has-win-controls' : '')} data-tauri-drag-region>
      <span data-tauri-drag-region className="brand-text text-[15px] font-semibold tracking-[-0.025em] text-text-primary">{strings.topbar.brandText}</span>

      <HostPill />

      <div data-tauri-drag-region className="spacer flex-1 min-w-[4px] self-stretch" />

      <button
        type="button"
        id="topbarSearch"
        className="cmd-btn flex h-[32px] min-w-[300px] shrink-0 items-center justify-between gap-[8px] rounded-full border border-border-subtle bg-bg-raised py-[5px] pl-[14px] pr-[8px] text-[12px] text-text-muted transition-all duration-fast ease-ease hover:border-border-default hover:bg-bg-hover hover:text-text-secondary max-1100:w-[32px] max-1100:min-w-0 max-1100:justify-center max-1100:p-0"
        title={strings.topbar.palette.title}
        onClick={() => openPalette()}
      >
        <span className="flex items-center gap-[8px]">
          <Search size={13} aria-hidden="true" />
          <span className="max-1100:hidden">{strings.topbar.palette.label}</span>
        </span>
        <span className="kbd inline-flex items-center rounded-full border border-border-default bg-bg-active px-[7px] py-[1px] font-mono text-[9.5px] font-medium leading-none text-text-secondary max-1100:hidden">
          {strings.topbar.palette.shortcut}
        </span>
      </button>

      <div data-tauri-drag-region className="spacer flex-1 min-w-[4px] self-stretch" />

      <ModeSwitch />

      <KillSwitch />

      <WindowControls />
    </header>
  );
}
