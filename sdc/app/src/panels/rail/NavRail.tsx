import { Building2, Moon, PanelLeft, PanelRight, Plug, Search, Settings, SquarePen, Sun } from 'lucide-react';

import { setTheme as applyTheme, type Theme } from '../../lib/theme';
import { strings } from '../../strings';
import { useKernelUi } from '../../store/kernelUi';
import { useLayoutStore } from '../../store/layout';
import { anchorBelow, useOverlayStore } from '../../store/overlays';
import { anyProviderNeedsAuth, useProviderStore } from '../../store/providers';
import { toast } from '../../store/toast';
import { BrandLogo } from '../ui/BrandLogo';
import { IconButton } from '../ui/IconButton';

/**
 * The navigation rail (0.19) - the window's spine, down its left edge.
 *
 * The 0.18 topbar carried thirteen controls in one row; the ones you reach for while working (a new chat,
 * search, the providers, the two panels) were the same size and weight as the ones you set once. The rail
 * takes the navigation: the mark at the top (About), then the verbs of the workspace, and the settings
 * pinned to the bottom where every desktop app keeps them. The topbar keeps only what describes the
 * present - which machine, which mode, the command search and Stop all.
 *
 * Every button keeps the id its topbar twin had, so keyboard maps, tests and probes still find it.
 */
export function NavRail() {
  const { theme, sidebar, right, toggleSidebar, toggleRight, setTheme } = useLayoutStore();
  const providers = useProviderStore((state) => state.providers);
  const openSettings = useOverlayStore((state) => state.openSettings);
  const openHub = useOverlayStore((state) => state.openHub);
  const openPalette = useOverlayStore((state) => state.openPalette);
  const openNewChat = useOverlayStore((state) => state.openNewChat);

  const needsAuth = anyProviderNeedsAuth(providers);
  const nextTheme: Theme = theme === 'light' ? 'dark' : 'light';

  const toggleTheme = (): void => {
    applyTheme(nextTheme);
    setTheme(nextTheme);
    toast(nextTheme === 'light' ? strings.topbar.theme.light : strings.topbar.theme.dark);
  };

  return (
    <nav className="navrail" aria-label={strings.rail.label}>
      <button
        type="button"
        className="brand-mark mb-[6px] grid h-[40px] w-[40px] place-items-center rounded-xl transition-transform duration-base ease-spring hover:scale-[1.06]"
        title={strings.topbar.brandTitle}
        aria-label={strings.topbar.brandTitle}
        onClick={() => openSettings('about')}
      >
        <BrandLogo size={32} glow />
      </button>

      <IconButton
        id="railNewChat"
        icon={SquarePen}
        label={strings.sidebar.newChat}
        size={40}
        iconSize={18}
        className="rail-btn rail-primary"
        onClick={(event) => openNewChat(anchorBelow(event.currentTarget))}
      />
      <IconButton id="openPalette" icon={Search} label={strings.topbar.palette.title} size={40} iconSize={18} className="rail-btn" onClick={() => openPalette()} />

      <span className="rail-sep" aria-hidden="true" />

      <IconButton id="toggleSidebar" icon={PanelLeft} label={strings.topbar.sidebar.title} size={40} iconSize={18} className="rail-btn" active={sidebar === 'visible'} onClick={toggleSidebar} />
      <IconButton id="toggleRight" icon={PanelRight} label={strings.topbar.right.title} size={40} iconSize={18} className="rail-btn" active={right === 'visible'} onClick={toggleRight} />

      <span className="rail-sep" aria-hidden="true" />

      <IconButton
        id="openProviders"
        icon={Plug}
        label={needsAuth ? strings.topbar.providers.dotTitle : strings.topbar.providers.title}
        hasDot={needsAuth}
        size={40}
        iconSize={18}
        className="rail-btn"
        onClick={() => openHub()}
      />
      <IconButton id="openAgency" icon={Building2} label={strings.kernel.agency.title} size={40} iconSize={18} className="rail-btn" onClick={() => useKernelUi.getState().openAgency()} />

      <span className="flex-1" />

      <IconButton id="themeToggle" icon={theme === 'light' ? Sun : Moon} label={strings.topbar.theme.title} size={40} iconSize={18} className="rail-btn" onClick={toggleTheme} />
      <IconButton id="openSettings" icon={Settings} label={strings.topbar.settings.title} size={40} iconSize={18} className="rail-btn" onClick={() => openSettings()} />
    </nav>
  );
}
