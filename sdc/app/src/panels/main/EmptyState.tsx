import { FolderOpen, Plug, Plus, ServerCog, Sparkles } from 'lucide-react';

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
      <div className="empty-icon mb-[8px] grid h-[56px] w-[56px] place-items-center rounded-lg text-accent shadow-[0_0_0_1px_var(--border-subtle),0_6px_20px_rgba(91,156,255,.12)] [background-image:linear-gradient(135deg,var(--accent-subtle),var(--purple-subtle))]">
        {hasProject ? (
          <Sparkles size={26} aria-hidden="true" />
        ) : (
          <FolderOpen size={26} aria-hidden="true" />
        )}
      </div>

      <div className="empty-title text-[16px] font-semibold text-text-primary">{title}</div>

      <div className="empty-desc max-w-[400px] text-[12.5px] leading-[1.6] text-text-muted">
        {description}
      </div>

      <div className="empty-chips mt-[10px] flex flex-wrap justify-center gap-[6px]">
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[6px] rounded-full border border-border-subtle bg-bg-raised px-[12px] py-[6px] text-[12px] text-text-secondary transition-all duration-fast ease-ease hover:-translate-y-px hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
          onClick={() => void openFolder()}
        >
          <FolderOpen size={12} aria-hidden="true" />
          {strings.main.noProject.action}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[6px] rounded-full border border-border-subtle bg-bg-raised px-[12px] py-[6px] text-[12px] text-text-secondary transition-all duration-fast ease-ease hover:-translate-y-px hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
          onClick={() => useOverlayStore.getState().openNewProject()}
        >
          <Sparkles size={12} aria-hidden="true" />
          {strings.scaffold.title}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[6px] rounded-full border border-border-subtle bg-bg-raised px-[12px] py-[6px] text-[12px] text-text-secondary transition-all duration-fast ease-ease hover:-translate-y-px hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
          onClick={(event) => openNewChat(anchorBelow(event.currentTarget))}
        >
          <Plus size={12} aria-hidden="true" />
          {strings.main.empty.chips.newChat}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[6px] rounded-full border border-border-subtle bg-bg-raised px-[12px] py-[6px] text-[12px] text-text-secondary transition-all duration-fast ease-ease hover:-translate-y-px hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
          onClick={() => openHub()}
        >
          <Plug size={12} aria-hidden="true" />
          {strings.main.empty.chips.connectModel}
        </button>
        <button
          type="button"
          className="empty-chip inline-flex items-center gap-[6px] rounded-full border border-border-subtle bg-bg-raised px-[12px] py-[6px] text-[12px] text-text-secondary transition-all duration-fast ease-ease hover:-translate-y-px hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
          onClick={() => openAddHost()}
        >
          <ServerCog size={12} aria-hidden="true" />
          {strings.main.empty.chips.addVps}
        </button>
      </div>
    </div>
  );
}
