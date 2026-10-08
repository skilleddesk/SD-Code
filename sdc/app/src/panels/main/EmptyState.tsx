import { FolderOpen, Plug, Plus, ServerCog, Sparkles } from 'lucide-react';

import { BrandLogo } from '../ui/BrandLogo';

import { strings } from '../../strings';
import { openFolder } from '../../store/intents';
import { anchorBelow, useOverlayStore } from '../../store/overlays';
import { useAppStore } from '../../store/store';

/**
 * The empty state of the main column - spec section 7.13, exact text.
 *
 *   No project        "Open a folder to get started"   [Open folder]
 *   No chat open      "Start a new chat, or pick one from the sidebar. Work across multiple chats and
 *                      hosts."                          [+ New chat] [Connect a model] [Add a VPS]
 *
 * The two states are one component on purpose. They are the same moment seen twice: a window with no
 * folder and no chat, and a window with a folder but no chat open. The first is the one a fresh install
 * lands in - and until 0.7.6 nothing in it could be done, because the app had no folder to work in, so
 * the whole state is new copy plus the button that leaves it.
 *
 * The chips are the ways out of an empty app: get a folder, start working, get a model, get a machine -
 * and each one opens a surface this step already has a store flag or an intent for.
 *
 * The description line is one string rather than three sentences glued together, because the spec
 * gives it as one line and a translator needs to see it that way.
 */
export function EmptyState() {
  const openNewChat = useOverlayStore((state) => state.openNewChat);
  const openHub = useOverlayStore((state) => state.openHub);
  const openAddHost = useOverlayStore((state) => state.openAddHost);
  /* A folder, any folder: the state is about *this window* having somewhere to work (0.7.6). */
  const hasProject = useAppStore((state) => state.projects.length > 0);

  const title = hasProject ? strings.main.empty.title : strings.main.noProject.title;
  const description = hasProject ? strings.main.empty.description : strings.main.noProject.description;

  return (
    <div className="empty-wrap flex flex-1 flex-col items-center justify-center gap-[10px] px-[24px] py-[40px] text-center">
      {/* 0.18: the mark itself, lit, above a headline in the brand light. */}
      <div className="empty-icon relative mb-[10px] grid place-items-center">
        <span className="absolute h-[120px] w-[120px] rounded-full opacity-60 blur-[38px] [background-image:var(--grad-brand)]" aria-hidden="true" />
        <BrandLogo size={76} glow className="relative" />
      </div>

      <div className="empty-title text-[22px] font-semibold tracking-[-0.025em] text-text-primary">
        {hasProject ? <span className="brand-text-fill">{title}</span> : title}
      </div>

      <div className="empty-desc max-w-[440px] text-[13px] leading-[1.65] text-text-secondary">
        {description}
      </div>

      <div className="empty-chips mt-[10px] flex flex-wrap justify-center gap-[6px]">
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[7px] rounded-full border border-border-default bg-bg-raised px-[14px] py-[7px] text-[12.5px] text-text-secondary shadow-sm transition-all duration-base ease-ease hover:-translate-y-[2px] hover:border-accent/40 hover:bg-bg-hover hover:text-text-primary hover:shadow-md"
          onClick={() => void openFolder()}
        >
          <FolderOpen size={12} aria-hidden="true" />
          {strings.main.noProject.action}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[7px] rounded-full border border-border-default bg-bg-raised px-[14px] py-[7px] text-[12.5px] text-text-secondary shadow-sm transition-all duration-base ease-ease hover:-translate-y-[2px] hover:border-accent/40 hover:bg-bg-hover hover:text-text-primary hover:shadow-md"
          onClick={() => useOverlayStore.getState().openNewProject()}
        >
          <Sparkles size={12} aria-hidden="true" />
          {strings.scaffold.title}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[7px] rounded-full border border-border-default bg-bg-raised px-[14px] py-[7px] text-[12.5px] text-text-secondary shadow-sm transition-all duration-base ease-ease hover:-translate-y-[2px] hover:border-accent/40 hover:bg-bg-hover hover:text-text-primary hover:shadow-md"
          onClick={(event) => openNewChat(anchorBelow(event.currentTarget))}
        >
          <Plus size={12} aria-hidden="true" />
          {strings.main.empty.chips.newChat}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[7px] rounded-full border border-border-default bg-bg-raised px-[14px] py-[7px] text-[12.5px] text-text-secondary shadow-sm transition-all duration-base ease-ease hover:-translate-y-[2px] hover:border-accent/40 hover:bg-bg-hover hover:text-text-primary hover:shadow-md"
          onClick={() => openHub()}
        >
          <Plug size={12} aria-hidden="true" />
          {strings.main.empty.chips.connectModel}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[7px] rounded-full border border-border-default bg-bg-raised px-[14px] py-[7px] text-[12.5px] text-text-secondary shadow-sm transition-all duration-base ease-ease hover:-translate-y-[2px] hover:border-accent/40 hover:bg-bg-hover hover:text-text-primary hover:shadow-md"
          onClick={() => openAddHost()}
        >
          <ServerCog size={12} aria-hidden="true" />
          {strings.main.empty.chips.addVps}
        </button>
      </div>
    </div>
  );
}
