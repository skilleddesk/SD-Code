import { Plug, Plus, ServerCog, Sparkles } from 'lucide-react';

import { strings } from '../../strings';
import { anchorBelow, useOverlayStore } from '../../store/overlays';

/**
 * The empty state of the main column - spec section 7.13, exact text.
 *
 *   No chat open
 *   Start a new chat, or pick one from the sidebar. Work across multiple chats and hosts.
 *   [+ New chat]  [Connect a model]  [Add a VPS]
 *
 * It is centred in whatever room is left, with a 56px sparkles tile above it and three pill chips
 * under it. The chips are the three ways out of an empty app: start working, get a model, get a
 * machine - and each one opens a surface this step already has a store flag for.
 *
 * The description line is one string rather than three sentences glued together, because the spec
 * gives it as one line and a translator needs to see it that way.
 */
export function EmptyState() {
  const openNewChat = useOverlayStore((state) => state.openNewChat);
  const openHub = useOverlayStore((state) => state.openHub);
  const openAddHost = useOverlayStore((state) => state.openAddHost);

  return (
    <div className="empty-wrap flex flex-1 flex-col items-center justify-center gap-[10px] px-[24px] py-[40px] text-center">
      <div className="empty-icon mb-[8px] grid h-[56px] w-[56px] place-items-center rounded-lg text-accent shadow-[0_0_0_1px_var(--border-subtle),0_6px_20px_rgba(91,156,255,.12)] [background-image:linear-gradient(135deg,var(--accent-subtle),var(--purple-subtle))]">
        <Sparkles size={26} aria-hidden="true" />
      </div>

      <div className="empty-title text-[16px] font-semibold text-text-primary">
        {strings.main.empty.title}
      </div>

      <div className="empty-desc max-w-[400px] text-[12.5px] leading-[1.6] text-text-muted">
        {strings.main.empty.description}
      </div>

      <div className="empty-chips mt-[10px] flex flex-wrap justify-center gap-[6px]">
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
