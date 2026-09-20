import { ChevronDown } from 'lucide-react';
import { useEffect, useRef } from 'react';

import { tierLabel, useModelStore, type Tier } from '../../store/model';
import { ModelDropdown } from './ModelDropdown';
import { tierIcon } from './modelIcons';

/**
 * `.model-selector` - the trigger in the prompt toolbar, and the owner of the dropdown's open state
 * (spec sections 7.6 and 9.3).
 *
 * The trigger is a mono pill that names all three choices at once:
 *
 *   [tier icon] Balanced · claude_code · sonnet  [chevron]
 *
 * The tier's icon chip is tinted by the tier - amber for Fast, accent for Balanced, purple for Deep
 * - which is the only place the tier is colour-coded, and it is why the icon map lives in
 * `ModelDropdown` next to the rows it has to match. At 900px the engine and the model drop out and
 * the tier stays, because the tier is the choice a narrow window can still make sense of.
 *
 * Outside-click dismissal is done here rather than in the dropdown because this element is the only
 * thing that is *not* "outside": a press anywhere in the trigger toggles, a press anywhere else
 * closes. The listener runs in the capture phase so that a press on another control closes the
 * dropdown *and* still does what that control does.
 */
const TIER_TONE: Record<Tier, string> = {
  fast: 'bg-orange-subtle text-state-waiting',
  balanced: 'bg-accent-subtle text-accent',
  deep: 'bg-purple-subtle text-purple',
};

export function ModelSelector() {
  const { tier, engine, model, dropdownOpen, toggleDropdown, closeDropdown } = useModelStore();
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

  const Icon = tierIcon(tier);

  return (
    <div className="model-selector relative" ref={rootRef}>
      <div
        className={
          'model-trigger flex h-[28px] cursor-pointer items-center gap-[8px] rounded-md border bg-bg-raised px-[10px] py-[5px] pl-[8px] font-mono text-[11.5px] transition-all duration-fast ease-ease ' +
          (dropdownOpen
            ? 'open border-border-focus bg-bg-overlay text-text-primary shadow-[0_0_0_3px_var(--accent-subtle)]'
            : 'border-border-subtle text-text-secondary hover:border-border-default hover:bg-bg-hover hover:text-text-primary')
        }
        role="button"
        tabIndex={0}
        aria-haspopup="dialog"
        aria-expanded={dropdownOpen}
        onClick={toggleDropdown}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            toggleDropdown();
          }
        }}
      >
        <span
          className={
            'tier-icon ' + tier + ' grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm ' + TIER_TONE[tier]
          }
        >
          <Icon size={11} aria-hidden="true" />
        </span>

        <span className="mlabel font-semibold text-text-primary">{tierLabel(tier)}</span>
        <span className="msep text-border-strong max-900:hidden">·</span>
        <span className="mengine text-text-secondary max-900:hidden">{engine}</span>
        <span className="msep text-border-strong max-900:hidden">·</span>
        <span className="mmodel text-text-muted max-900:hidden">{model}</span>

        <ChevronDown
          size={12}
          aria-hidden="true"
          className={
            'mchev ml-[2px] text-text-muted transition-transform duration-200 ease-ease ' +
            (dropdownOpen ? 'rotate-180' : '')
          }
        />
      </div>

      {/*
        The trigger names the tier, so the accessible name needs the rest of the sentence - which is
        the same three values, spelled out.
      */}
      <span className="sr-only">
        {tierLabel(tier)} · {engine} · {model}
      </span>

      {dropdownOpen ? <ModelDropdown /> : null}
    </div>
  );
}
