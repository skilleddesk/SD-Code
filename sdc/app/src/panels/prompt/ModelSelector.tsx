import { ChevronDown } from 'lucide-react';
import { useEffect, useRef } from 'react';

import { useChatModel } from '../../store/chatModel';
import { useModelStore } from '../../store/model';
import { ModelDropdown } from './ModelDropdown';
import { engineIcon } from './modelIcons';

/**
 * `.model-selector` - the trigger in the prompt toolbar, and the owner of the dropdown's open state.
 *
 * The trigger names one thing, the model, the way a person reads it:
 *
 *   [engine icon] Claude Opus 5.5  [chevron]
 *
 * The readable name comes from the catalogue row; the raw id is the tooltip, and the trigger falls back to it
 * while the catalogue has not arrived.
 *
 * Outside-click dismissal is done here rather than in the dropdown because this element is the only
 * thing that is *not* "outside": a press anywhere in the trigger toggles, a press anywhere else
 * closes. The listener runs in the capture phase so that a press on another control closes the
 * dropdown *and* still does what that control does.
 */
export function ModelSelector({ sessionId }: { sessionId: string | null }) {
  /* The chat this box belongs to - each chat has its own model (0.16.1). */
  const { engine, model, providerId } = useChatModel(sessionId);
  const key = sessionId ?? '';
  const dropdownOpen = useModelStore((state) => state.dropdownOpen === key);
  const closeDropdown = useModelStore((state) => state.closeDropdown);
  const name = useModelStore(
    (state) =>
      state.catalog.find((row) => row.id === model && (providerId === null || row.providerId === providerId))?.name ??
      state.catalog.find((row) => row.id === model)?.name,
  );
  const toggleDropdown = (): void => useModelStore.getState().toggleDropdown(key);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!dropdownOpen) {
      return;
    }

    const handlePointerDown = (event: PointerEvent): void => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        closeDropdown();
      }
    };

    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        closeDropdown();
      }
    };

    document.addEventListener('pointerdown', handlePointerDown, true);
    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [dropdownOpen, closeDropdown]);

  const Icon = engineIcon(engine);
  const label = name === undefined || name === '' ? model : name;

  return (
    <div className="model-selector relative" ref={rootRef}>
      <div
        className={
          'model-trigger group flex h-[34px] min-w-0 max-w-[240px] cursor-pointer items-center gap-[8px] rounded-full border py-[3px] pl-[3px] pr-[10px] text-[12px] outline-none transition-all duration-fast ease-ease focus-visible:ring-2 focus-visible:ring-accent/50 max-600:max-w-[170px] ' +
          (dropdownOpen
            ? 'open border-border-strong bg-bg-active text-text-primary'
            : 'border-border-subtle bg-bg-raised text-text-secondary hover:border-border-default hover:bg-bg-hover hover:text-text-primary')
        }
        role="button"
        tabIndex={0}
        aria-haspopup="dialog"
        aria-label={label}
        title={model}
        aria-expanded={dropdownOpen}
        onClick={toggleDropdown}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            toggleDropdown();
          }
        }}
      >
        <span className="brand-fill grid h-[26px] w-[26px] shrink-0 place-items-center rounded-full" aria-hidden="true">
          <Icon size={13} />
        </span>
        <span className="mmodel min-w-0 truncate font-medium text-text-primary">{label}</span>
        <ChevronDown
          size={12}
          aria-hidden="true"
          className={'mchev shrink-0 text-text-muted transition-transform duration-200 ease-ease ' + (dropdownOpen ? 'rotate-180' : '')}
        />
      </div>

      {dropdownOpen ? <ModelDropdown sessionId={sessionId} /> : null}
    </div>
  );
}
