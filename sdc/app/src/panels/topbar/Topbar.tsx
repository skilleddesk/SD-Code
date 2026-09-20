import { Moon, PanelLeft, PanelRight, Plug, Search, Settings, Sun } from 'lucide-react';

import { strings } from '../../strings';
import { setTheme as applyTheme, type Theme } from '../../lib/theme';
import { useLayoutStore } from '../../store/layout';
import { useOverlayStore } from '../../store/overlays';
import { anyProviderNeedsAuth, useProviderStore } from '../../store/providers';
import { toast } from '../../store/toast';
import { HostPill } from './HostPill';
import { IconButton } from './IconButton';
import { ModeSwitch } from './ModeSwitch';
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
  const { theme, toggleSidebar, toggleRight, setTheme } = useLayoutStore();
  const providers = useProviderStore((state) => state.providers);
  const openSettings = useOverlayStore((state) => state.openSettings);
  const openHub = useOverlayStore((state) => state.openHub);
  const openPalette = useOverlayStore((state) => state.openPalette);

  const needsAuth = anyProviderNeedsAuth(providers);
  const nextTheme: Theme = theme === 'light' ? 'dark' : 'light';

  const toggleTheme = (): void => {
    applyTheme(nextTheme);
    setTheme(nextTheme);
    toast(nextTheme === 'light' ? strings.topbar.theme.light : strings.topbar.theme.dark);
  };

  return (
    <header className="topbar">
      <div className="brand flex items-center gap-[8px] pr-[4px] shrink-0 font-semibold text-[13px]">
        <button
          type="button"
          className="brand-mark grid place-items-center w-[24px] h-[24px] rounded-[6px] text-on-accent text-[12px] font-bold [background-image:var(--grad-brand)] shadow-[0_0_0_1px_rgba(255,255,255,.08),0_0_16px_rgba(91,156,255,.3)]"
          title={strings.topbar.brandTitle}
          aria-label={strings.topbar.brandTitle}
          onClick={() => openSettings('about')}
        >
          {strings.topbar.brandInitial}
        </button>
        <span className="brand-text">{strings.topbar.brandText}</span>
      </div>

      <HostPill />

      <div className="spacer flex-1 min-w-[4px]" />

      <ModeSwitch />

      <button
        type="button"
        id="openPalette"
        className="cmd-btn flex items-center gap-[8px] px-[10px] py-[5px] pl-[12px] h-[28px] shrink-0 rounded-md bg-bg-raised border border-border-subtle text-text-muted text-[11.5px] justify-between min-w-[220px] transition-all duration-fast ease-ease hover:border-border-default hover:bg-bg-hover hover:text-text-secondary max-1100:min-w-0 max-1100:w-[28px] max-1100:p-0 max-1100:justify-center"
        title={strings.topbar.palette.title}
        onClick={() => openPalette()}
      >
        <span className="flex items-center gap-[8px]">
          <Search size={12} aria-hidden="true" />
          <span className="max-1100:hidden">{strings.topbar.palette.label}</span>
        </span>
        <span className="kbd inline-flex items-center px-[5px] py-[1px] rounded-[3px] bg-bg-base border border-border-default border-b-2 font-mono text-[9.5px] font-medium leading-none text-text-secondary max-1100:hidden">
          {strings.topbar.palette.shortcut}
        </span>
      </button>

      <IconButton
        id="openProviders"
        icon={Plug}
        label={needsAuth ? strings.topbar.providers.dotTitle : strings.topbar.providers.title}
        hasDot={needsAuth}
        onClick={() => openHub()}
      />

      <IconButton
        id="themeToggle"
        icon={theme === 'light' ? Sun : Moon}
        label={strings.topbar.theme.title}
        onClick={toggleTheme}
      />

      <IconButton
        id="toggleSidebar"
        icon={PanelLeft}
        label={strings.topbar.sidebar.title}
        onClick={toggleSidebar}
      />

      <IconButton
        id="toggleRight"
        icon={PanelRight}
        label={strings.topbar.right.title}
        onClick={toggleRight}
      />

      <IconButton
        id="openSettings"
        icon={Settings}
        label={strings.topbar.settings.title}
        onClick={() => openSettings()}
      />
    </header>
  );
}
