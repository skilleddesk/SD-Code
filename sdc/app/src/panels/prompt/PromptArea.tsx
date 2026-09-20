import {
  AtSign,
  CornerDownLeft,
  Database,
  Hash,
  Image as ImageIcon,
  Paperclip,
  Slash,
} from 'lucide-react';
import { useRef, type KeyboardEvent } from 'react';

import { strings } from '../../strings';
import { tierLabel, useModelStore } from '../../store/model';
import { toast } from '../../store/toast';
import { IconButton } from '../ui/IconButton';
import { ModelSelector } from './ModelSelector';
import { QueuedChips } from './QueuedChips';

/**
 * `.prompt-area` - the input, and everything around it (spec section 7.6).
 *
 * Four rows, top to bottom:
 *
 *   .prompt-toolbar  the model selector, then two chips - `1 file` and `12.4k ctx` - which say what
 *                    this turn will carry besides the text
 *   .queued-chips    up to three steering prompts (section 9.7)
 *   .prompt-box      the textarea and its action row: four tool buttons on the left, a tier/model
 *                    hint and the Send button on the right
 *   .tip-line        `@` reference file · `/` commands · `⌘K` palette, which folds away at 900px
 *
 * The textarea grows with its content up to 200px and then scrolls. That is done by hand
 * (`height: auto`, then `scrollHeight`) rather than with a dependency: `field-sizing: content` would
 * be the modern answer, but WebView2 and WKWebView do not both have it yet.
 *
 * Keys (spec section 9.1): Enter sends, Shift+Enter is a newline, Escape interrupts - which is a
 * toast until an engine is actually running a turn. Sending announces itself the way the prototype
 * does: `Sent to <engine> · <model>`.
 *
 * The inner column is capped at 780px and centred, so a prompt does not stretch to the width of a
 * wide monitor.
 */
const CHIP =
  'chip inline-flex h-[28px] items-center gap-[5px] rounded-md border border-border-subtle bg-bg-raised px-[8px] py-[3px] font-mono text-[11px] text-text-secondary';

const KBD =
  'rounded-[3px] border border-border-subtle bg-bg-raised px-[5px] py-[1px] font-mono text-[10px] text-text-secondary';

export function PromptArea() {
  const { tier, engine, model } = useModelStore();
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  /** Grow the box to fit its content, up to the 200px ceiling the spec sets. */
  const grow = (): void => {
    const textarea = textareaRef.current;

    if (!textarea) {
      return;
    }

    textarea.style.height = 'auto';
    textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
  };

  const send = (): void => {
    const textarea = textareaRef.current;
    const text = textarea?.value.trim() ?? '';

    if (!textarea || text === '') {
      toast(strings.prompt.empty);
      return;
    }

    textarea.value = '';
    textarea.style.height = 'auto';
    toast(strings.prompt.sent(engine, model));
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      send();
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      toast(strings.prompt.interrupt);
    }
  };

  return (
    <div className="prompt-area shrink-0 border-t border-border-subtle bg-bg-base px-[24px] pb-[12px] pt-[10px] max-600:px-[14px]">
      <div className="prompt-inner mx-auto max-w-[780px]">
        <div className="prompt-toolbar mb-[8px] flex flex-wrap items-center gap-[6px]">
          <ModelSelector />

          <span className={CHIP}>
            <Hash size={10} aria-hidden="true" />
            <span>{strings.prompt.filesChip}</span>
          </span>

          <span className={CHIP}>
            <Database size={10} aria-hidden="true" />
            <span>{strings.prompt.contextChip}</span>
          </span>
        </div>

        <QueuedChips />

        <div className="prompt-box flex flex-col gap-[8px] rounded-lg border border-border-default bg-bg-input px-[13px] py-[11px] transition-all duration-base ease-ease focus-within:border-border-focus focus-within:shadow-[0_0_0_3px_var(--accent-subtle),0_6px_20px_var(--accent-subtle)]">
          <textarea
            ref={textareaRef}
            rows={1}
            className="max-h-[200px] min-h-[22px] w-full resize-none bg-transparent text-[14px] leading-[1.55] text-text-primary placeholder:text-text-muted"
            placeholder={strings.prompt.placeholder}
            aria-label={strings.prompt.placeholder}
            onInput={grow}
            onKeyDown={handleKeyDown}
          />

          <div className="prompt-actions flex items-center gap-[6px]">
            <div className="toolbar flex gap-[2px]">
              <IconButton
                icon={Paperclip}
                label={strings.prompt.toolbar.attach}
                iconSize={14}
                onClick={() => toast(strings.prompt.toolbar.attach)}
              />
              <IconButton
                icon={ImageIcon}
                label={strings.prompt.toolbar.image}
                iconSize={14}
                onClick={() => toast(strings.prompt.toolbar.image)}
              />
              <IconButton
                icon={AtSign}
                label={strings.prompt.toolbar.file}
                iconSize={14}
                onClick={() => toast(strings.prompt.toolbar.file)}
              />
              <IconButton
                icon={Slash}
                label={strings.prompt.toolbar.command}
                iconSize={14}
                onClick={() => toast(strings.prompt.toolbar.command)}
              />
            </div>

            <div className="context-hint ml-auto flex items-center gap-[8px] font-mono text-[10.5px] text-text-muted max-700:hidden">
              <span>
                {tierLabel(tier)} · {model}
              </span>
            </div>

            <button
              type="button"
              className="send-btn flex items-center gap-[6px] rounded-md bg-accent px-[13px] py-[6px] text-[12px] font-semibold text-text-on-accent transition-all duration-fast ease-ease hover:bg-accent-hover hover:shadow-[0_3px_12px_var(--accent-glow)] active:scale-[.97]"
              onClick={send}
            >
              {strings.prompt.send}
              <CornerDownLeft size={12} aria-hidden="true" />
            </button>
          </div>
        </div>

        <div className="tip-line mt-[6px] text-center text-[10.5px] text-text-muted max-900:hidden">
          <kbd className={KBD}>{strings.prompt.tip.file}</kbd> {strings.prompt.tip.fileLabel} ·{' '}
          <kbd className={KBD}>{strings.prompt.tip.command}</kbd> {strings.prompt.tip.commandLabel} ·{' '}
          <kbd className={KBD}>{strings.prompt.tip.palette}</kbd> {strings.prompt.tip.paletteLabel}
        </div>
      </div>
    </div>
  );
}
