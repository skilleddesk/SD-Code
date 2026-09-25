import {
  CornerDownLeft,
  Square,
  Database,
  Hash,
  Image as ImageIcon,
  Paperclip,
} from 'lucide-react';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';

import { strings } from '../../strings';
import { pickFiles, type PickKind, type PickedFile } from '../../lib/picker';
import { interruptTurn, sendPrompt } from '../../store/intents';
import { useAppStore } from '../../store/store';
import { useModelStore } from '../../store/model';
import { toast } from '../../store/toast';
import { IconButton } from '../ui/IconButton';
import { ComposeSwitch } from './ComposeSwitch';
import { FolderChip } from './FolderChip';
import { ModelSelector } from './ModelSelector';
import { QueuedChips } from './QueuedChips';

/**
 * `.prompt-area` - the input, and everything around it (spec section 7.6).
 *
 * Three rows, top to bottom:
 *
 *   .prompt-toolbar  the model selector, then the two chips - `1 file` and `12.4k ctx` - which say
 *                    what this turn will carry besides the text, and which are absent when there is
 *                    nothing to carry
 *   .queued-chips    up to three steering prompts (section 9.7)
 *   .prompt-box      the textarea and its action row: four tool buttons on the left, the Send button
 *                    on the right
 *
 * **Two rows were removed in 0.7.0, and the reason is a report**: a faint `Balanced · claude_code ·
 * sonnet` line sat inside the box under the Send button, and a row of tag-shaped `@` / `/` / `⌘K`
 * chips sat under the box. Both said things twice - the model line is exactly what the selector one
 * row above already says, in the same words, and the chips advertised an `@` picker and a `/` command
 * list that do not exist. Two faint rows of decoration in the place a person types is noise, and the
 * instruction was to take it out of every box. What is left is what works.
 *
 * **The two toolbar buttons became real in 0.7.5.** The report was *"GUI file-picker nai"*, and it was
 * exact: the paperclip and the image button called `toast(...)` and opened nothing. They now open the
 * native dialog (`lib/picker.ts`) and put what was picked into the prompt as `@<path>`.
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

export interface PromptAreaProps {
  /** The chat this box sends to - in split view there are two, and each pane's box is its own chat's. */
  sessionId?: string;
}

export function PromptArea({ sessionId }: PromptAreaProps = {}) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  /* This chat's turn that is still running, if any: while there is one, Send becomes Stop. */
  const running = useAppStore((state) => {
    for (let index = state.turns.length - 1; index >= 0; index -= 1) {
      const turn = state.turns[index];

      if (turn !== undefined && turn.sessionId === sessionId) {
        return turn.status === 'running' || turn.status === 'stuck' ? turn.id : null;
      }
    }

    return null;
  });

  /*
   * What this prompt will carry besides the text - and it starts at nothing, because that is the
   * truth for a window that has attached nothing. `attached` is filled by the paperclip and the image
   * button (`lib/picker.ts`, a real dialog), and `tokens` is the context the daemon reports for the
   * last turn of this chat, which a fresh window has not run. Both chips are therefore absent on first
   * run: no decoration, no invented `12.4k`.
   */
  const [attached, setAttached] = useState<readonly PickedFile[]>([]);
  const draft = useModelStore((state) => state.draft);

  /* The chat's turn ended: the next queued prompt for it goes out now. */
  useEffect(() => {
    if (running !== null || sessionId === undefined) {
      return;
    }

    const next = useModelStore.getState().takeNext(sessionId);

    if (next !== null) {
      void sendPrompt(next, sessionId);
    }
  }, [running, sessionId]);

  /* A prompt handed over from elsewhere ("Fix with a prompt"): into the box, focused, ready to edit. */
  useEffect(() => {
    const textarea = textareaRef.current;

    if (draft === null || textarea === null) {
      return;
    }

    textarea.value = draft;
    textarea.focus();
    textarea.setSelectionRange(draft.length, draft.length);
    textarea.style.height = 'auto';
    textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
    useModelStore.getState().setDraft(null);
  }, [draft]);
  const context = { files: attached.length, tokens: null as number | null };

  /**
   * The two toolbar buttons, which used to be toasts (`Attach a file`, `Paste image`) and are now the
   * dialog itself (`lib/picker.ts`).
   *
   * A picked file goes into the prompt as `@<path>`, not into a chip of its own, and that is a fact
   * about the protocol rather than a preference: `engine.start` carries the prompt and nothing else, so
   * a path *inside the prompt* is a reference the engine can open today, while an attachment chip would
   * be decoration until attachments travel with the turn. The `1 file` chip beside the model selector
   * counts what was picked, so the button's work is visible even before Send.
   */
  const attach = (kind: PickKind): void => {
    void pickFiles(kind)
      .then((picked) => {
        const textarea = textareaRef.current;

        if (picked.length === 0 || textarea === null) {
          return;
        }

        const references = picked.map((file) => `@${file.path}`).join(' ');
        const present = textarea.value.trimEnd();

        textarea.value = present === '' ? `${references} ` : `${present} ${references} `;
        textarea.focus();
        grow();
        setAttached((current) => [...current, ...picked]);
      })
      .catch((error: unknown) => {
        /* A dialog that cannot open is a real failure (a missing capability, a broken plugin) and must
           not look like a user who changed their mind. */
        toast(error instanceof Error ? error.message : strings.prompt.pickFailed);
      });
  };

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

    /*
     * The send path, which until now was a toast.
     *
     * `sendPrompt` opens a session if the window has none, then calls the daemon's `engine.start` -
     * the same call the palette and `Fix this` make. It clears the box only once the daemon has
     * accepted the turn, so a send that fails leaves the words where they were; and it says what
     * happened (`Sent to …`, or the daemon's own error) because a button that looks like it worked is
     * worse than one that admits it did not.
     */
    const prompt = text;

    /* This chat's turn is still running: the prompt waits its turn instead of racing it. */
    if (running !== null && sessionId !== undefined) {
      if (!useModelStore.getState().enqueue(sessionId, prompt)) {
        toast(strings.prompt.queued.full);

        return;
      }

      textarea.value = '';
      textarea.style.height = 'auto';
      toast(strings.prompt.queued.queuedToast);

      return;
    }

    textarea.value = '';
    textarea.style.height = 'auto';
    /* The picked paths travelled *inside* `prompt`, so the count is about the next turn and starts
       again at nothing. */
    setAttached([]);

    void sendPrompt(prompt, sessionId).then((turnId) => {
      if (turnId === null) {
        /* Nothing was accepted, so the words go back: a send that quietly ate the prompt would be the
           same lie as a Send button that only toasts. */
        textarea.value = prompt;
        grow();
      }
    });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      send();
      return;
    }

    /* Escape is the global `turn.interrupt` command (commands/registry.ts), which stops this chat's
       running turn for real; this box used to catch it first and only toast "Interrupted". */
  };

  return (
    <div className="prompt-area shrink-0 border-t border-border-subtle bg-bg-base px-[24px] pb-[12px] pt-[10px] max-600:px-[14px]">
      <div className="prompt-inner mx-auto max-w-[780px]">
        <div className="prompt-toolbar mb-[8px] flex flex-wrap items-center gap-[6px]">
          <ComposeSwitch />

          <ModelSelector />

          {/* Which folder this chat works in (0.7.6) - and the way to change it. */}
          <FolderChip />

          {/*
            The two context chips, and the reason they are conditional.
            They used to be fixed strings - `1 file` and `12.4k ctx` - printed under every prompt in
            every window, including an empty one, because they were the prototype's decoration. A
            window that says it will carry a file it has not got is lying about the next turn, so
            each chip now waits for a real count: the daemon reports the context it loaded, and an
            attachment shows up when `@` actually attaches one.
          */}
          {context.files > 0 ? (
            <span className={CHIP}>
              <Hash size={10} aria-hidden="true" />
              <span>{strings.prompt.filesChip(context.files)}</span>
            </span>
          ) : null}

          {context.tokens === null ? null : (
            <span className={CHIP}>
              <Database size={10} aria-hidden="true" />
              <span>{strings.prompt.contextChip(context.tokens)}</span>
            </span>
          )}
        </div>

        <QueuedChips sessionId={sessionId} />

        <div className="prompt-box flex flex-col gap-[8px] rounded-lg border border-border-default bg-bg-input px-[13px] py-[11px] transition-all duration-base ease-ease focus-within:border-border-strong">
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
                onClick={() => attach('file')}
              />
              <IconButton
                icon={ImageIcon}
                label={strings.prompt.toolbar.image}
                iconSize={14}
                onClick={() => attach('image')}
              />
            </div>

            {running === null ? (
              <button
                type="button"
                className="send-btn ml-auto flex items-center gap-[6px] rounded-md bg-accent-fill px-[13px] py-[6px] text-[12px] font-semibold text-text-on-accent transition-all duration-fast ease-ease hover:bg-accent-hover hover:shadow-[0_3px_12px_var(--accent-glow)] active:scale-[.97]"
                onClick={send}
              >
                {strings.prompt.send}
                <CornerDownLeft size={12} aria-hidden="true" />
              </button>
            ) : (
              <button
                type="button"
                className="stop-btn ml-auto flex items-center gap-[6px] rounded-md border border-state-error bg-red-subtle px-[13px] py-[6px] text-[12px] font-semibold text-state-error transition-all duration-fast ease-ease hover:bg-state-error hover:text-text-on-accent active:scale-[.97]"
                title={strings.prompt.stopHint}
                onClick={() => void interruptTurn(running)}
              >
                <Square size={11} aria-hidden="true" fill="currentColor" />
                {strings.prompt.stop}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
