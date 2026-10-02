import {
  ArrowUp,
  Square,
  Hash,
  Image as ImageIcon,
  Paperclip,
  X,
} from 'lucide-react';
import { useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from 'react';

import { strings } from '../../strings';
import { pickFiles, type PickKind, type PickedFile } from '../../lib/picker';
import { interruptTurn, sendPrompt, steerTurn, type TurnImage } from '../../store/intents';
import { findFiles, listCommands } from '../../store/agentIntents';
import { ComposerMenu, type MenuItem } from './ComposerMenu';
import { ContextChip, ProcessesChip } from './ComposerChips';
import { QuestionCard } from './QuestionCard';
import { runSlash, type SlashOutcome } from './slash';
import { useAppStore } from '../../store/store';
import { useModelStore } from '../../store/model';
import { toast } from '../../store/toast';
import { IconButton } from '../ui/IconButton';
import { FolderChip } from './FolderChip';
import { ModelSelector } from './ModelSelector';
import { QueuedChips } from './QueuedChips';
import { IntentCard } from '../../kernel/IntentCard';
import { ResearchCard } from './ResearchCard';
import { VoiceButton } from '../../kernel/VoiceButton';
import { usePrefsStore } from '../../store/prefs';

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
  'chip inline-flex h-[26px] items-center gap-[5px] rounded-full px-[9px] font-mono text-[11px] text-text-muted';

/** The largest image a turn takes (0.13); a provider refuses bigger ones anyway. */
const IMAGE_CAP = 10 * 1024 * 1024;

/** Commands that run as soon as they are chosen: they take no words after them. */
const IMMEDIATE = new Set(['compact', 'memory', 'clear', 'help', 'review', 'init']);

/** The open `/` or `@` list: what it lists, where its word starts in the box, and the highlighted row. */
interface MenuState {
  kind: 'slash' | 'mention';
  query: string;
  start: number;
  items: MenuItem[];
  index: number;
  loading: boolean;
}

/** An image read in the window, as a turn carries it. */
function readImage(file: File): Promise<TurnImage | null> {
  return new Promise((resolve) => {
    if (file.size > IMAGE_CAP) {
      toast(strings.agent.images.tooBig);
      resolve(null);

      return;
    }

    const reader = new FileReader();

    reader.onload = () => {
      const url = String(reader.result ?? '');
      const data = url.split('base64,')[1] ?? '';

      resolve(data === '' ? null : { name: file.name || 'pasted.png', mediaType: file.type || 'image/png', data });
    };
    reader.onerror = () => resolve(null);
    reader.readAsDataURL(file);
  });
}

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
  /* Images for the next turn (0.13): pasted, dropped or picked - sent with it, then cleared. */
  const [images, setImages] = useState<readonly TurnImage[]>([]);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const menuRequest = useRef(0);
  /* The chat whose Intent Contract card this box shows (0.12): the pane's own, or the open one. */
  const activeTab = usePrefsStore((state) => state.activeTab);
  const cardSession = sessionId ?? activeTab;
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
  const context = { files: attached.length };

  /**
   * The `/` and `@` lists (0.13). A `/` at the very start opens the commands; an `@` after a space (or
   * at the start) opens the chat's files. The word being typed filters them; a space ends it.
   */
  const updateMenu = (): void => {
    const textarea = textareaRef.current;

    if (textarea === null) {
      return;
    }

    const value = textarea.value;
    const caret = textarea.selectionStart ?? value.length;
    const before = value.slice(0, caret);
    const request = ++menuRequest.current;

    if (/^\/[\w-]*$/.test(before)) {
      const query = before.slice(1).toLowerCase();

      void listCommands(sessionId ?? null).then((commands) => {
        if (request !== menuRequest.current) {
          return;
        }

        const items = commands
          .filter((command) => command.name.toLowerCase().startsWith(query) || (query.length > 1 && command.name.toLowerCase().includes(query)))
          .slice(0, 12)
          .map((command) => ({
            key: `${command.source}:${command.name}`,
            label: `/${command.name}`,
            detail: command.description,
            ...(command.source === 'sdc' ? {} : { badge: strings.agent.commands.fromProject }),
          }));

        setMenu({ kind: 'slash', query, start: 0, items, index: 0, loading: false });
      });

      return;
    }

    const mention = /(^|\s)@([^\s@]*)$/.exec(before);

    if (mention !== null && sessionId !== undefined) {
      const query = mention[2] ?? '';
      const start = caret - query.length - 1;

      setMenu((current) => ({ kind: 'mention', query, start, items: current?.kind === 'mention' ? current.items : [], index: 0, loading: true }));

      window.setTimeout(() => {
        if (request !== menuRequest.current) {
          return;
        }

        void findFiles(sessionId, query === '' ? '.' : query).then((files) => {
          if (request !== menuRequest.current) {
            return;
          }

          setMenu({
            kind: 'mention',
            query,
            start,
            items: files.map((file) => ({ key: file.path, label: file.path.split('/').pop() ?? file.path, detail: file.path })),
            index: 0,
            loading: false,
          });
        });
      }, 140);

      return;
    }

    setMenu(null);
  };

  /** A row of the open list, chosen: a command fills the box (or runs), a file becomes `@path`. */
  const choose = (index: number): void => {
    const textarea = textareaRef.current;
    const item = menu?.items[index];

    if (textarea === null || menu === null || item === undefined) {
      return;
    }

    if (menu.kind === 'slash') {
      textarea.value = `${item.label} `;
      setMenu(null);

      if (IMMEDIATE.has(item.label.slice(1)) && item.badge === undefined) {
        send();

        return;
      }
    } else {
      const caret = textarea.selectionStart ?? textarea.value.length;

      textarea.value = `${textarea.value.slice(0, menu.start)}@${item.detail} ${textarea.value.slice(caret)}`;
      setMenu(null);
    }

    textarea.focus();
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    grow();
  };

  /** A pasted screenshot becomes an image of the next turn, not a file path or nothing at all. */
  const paste = (event: ClipboardEvent<HTMLTextAreaElement>): void => {
    const files = Array.from(event.clipboardData?.files ?? []).filter((file) => file.type.startsWith('image/'));

    if (files.length === 0) {
      return;
    }

    event.preventDefault();

    void Promise.all(files.map(readImage)).then((read) => {
      const fresh = read.filter((image): image is TurnImage => image !== null);

      if (fresh.length > 0) {
        setImages((current) => [...current, ...fresh].slice(0, 8));
        toast(strings.agent.images.pasted);
      }
    });
  };

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

        /* An image is shown to the model (0.13), not referenced as a path it would have to open. */
        if (kind === 'image') {
          setImages((current) =>
            [
              ...current,
              ...picked.map((file) => ({ name: file.name, mediaType: mediaTypeOf(file.path), path: file.path })),
            ].slice(0, 8),
          );
          textarea.focus();

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

    /* This chat's turn is still running. First the words go **into** it (0.12.5): an agent turn reads
       them between steps and changes course, as Claude Code does. A turn that cannot take them (a CLI
       turn) leaves them queued as the next turn, as before. */
    if (running !== null && sessionId !== undefined) {
      textarea.value = '';
      textarea.style.height = 'auto';

      void steerTurn(running, prompt).then((accepted) => {
        if (accepted) {
          toast(strings.prompt.queued.steered);

          return;
        }

        if (!useModelStore.getState().enqueue(sessionId, prompt)) {
          toast(strings.prompt.queued.full);
          useModelStore.getState().setDraft(prompt);

          return;
        }

        toast(strings.prompt.queued.queuedToast);
      });

      return;
    }

    textarea.value = '';
    textarea.style.height = 'auto';
    setMenu(null);
    /* The picked paths travelled *inside* `prompt`, so the count is about the next turn and starts
       again at nothing. */
    setAttached([]);

    const withImages = images;

    setImages([]);

    /* A `/` command (0.13) runs here or becomes the words that are sent. */
    const outgoing: Promise<SlashOutcome> = prompt.startsWith('/') ? runSlash(prompt, sessionId ?? null) : Promise.resolve({ kind: 'send', prompt });

    void outgoing.then((outcome) => {
      if (outcome.kind === 'handled') {
        return;
      }

      void sendPrompt(outcome.prompt, sessionId, {
        images: [...withImages],
        ...(outcome.compact === true ? { compact: true } : {}),
      }).then((turnId) => {
        if (turnId === null) {
          /* Nothing was accepted, so the words go back: a send that quietly ate the prompt would be the
             same lie as a Send button that only toasts. */
          textarea.value = prompt;
          setImages(withImages);
          grow();
        }
      });
    });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    /* An input method still composing (Bengali, Chinese, Japanese…) owns Enter: it commits the word, it does not send (0.12). */
    if (event.nativeEvent.isComposing || event.keyCode === 229) {
      return;
    }

    /* The open `/` or `@` list takes the arrows, Enter, Tab and Escape (0.13). */
    if (menu !== null && menu.items.length > 0) {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        setMenu({ ...menu, index: (menu.index + (event.key === 'ArrowDown' ? 1 : menu.items.length - 1)) % menu.items.length });

        return;
      }

      if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') {
        event.preventDefault();
        choose(menu.index);

        return;
      }
    }

    if (menu !== null && event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      setMenu(null);

      return;
    }

    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      send();
      return;
    }

    /* Escape is the global `turn.interrupt` command (commands/registry.ts), which stops this chat's
       running turn for real; this box used to catch it first and only toast "Interrupted". */
  };

  const typing = running !== null;

  return (
    <div className="prompt-area relative shrink-0 bg-bg-base px-[24px] pb-[16px] pt-[8px] max-600:px-[14px]">
      {/* The conversation fades into the composer instead of ending on a hard edge. */}
      <div className="pointer-events-none absolute inset-x-0 -top-[24px] h-[24px] bg-gradient-to-t from-bg-base to-transparent" aria-hidden="true" />
      <div className="prompt-inner mx-auto max-w-[780px]">
        {cardSession === null ? null : <IntentCard sessionId={cardSession} />}
        {cardSession === null ? null : <QuestionCard sessionId={cardSession} />}
        <ResearchCard
          sessionId={cardSession}
          onCancel={(text) => {
            const textarea = textareaRef.current;

            if (textarea !== null) {
              textarea.value = text;
              textarea.focus();
              grow();
            }
          }}
        />

        <QueuedChips sessionId={sessionId} />

        {/*
          The composer (0.12.5): one box with everything in it - the words on top, and under them the
          attachments, Chat / Agent, the model and the folder, then Send. The controls used to sit in a
          row above the box, which read as a second toolbar rather than as part of what is being sent.
        */}
        <div className="relative">
        {menu === null ? null : (
          <ComposerMenu kind={menu.kind} items={menu.items} index={menu.index} loading={menu.loading} onChoose={choose} onHover={(index) => setMenu({ ...menu, index })} />
        )}
        <div
          className={
            'prompt-box rounded-[18px] border bg-bg-raised shadow-[0_8px_28px_-12px_rgba(0,0,0,.45)] transition-[border-color,box-shadow] duration-base ease-ease ' +
            (typing
              ? 'border-accent/40 shadow-[0_0_0_1px_var(--accent-glow),0_8px_28px_-12px_rgba(0,0,0,.45)]'
              : 'border-border-default hover:border-border-strong focus-within:!border-accent/50 focus-within:shadow-[0_0_0_4px_color-mix(in_srgb,var(--border-focus)_14%,transparent),0_8px_28px_-12px_rgba(0,0,0,.45)]')
          }
        >
          {/* No ring of its own: the box around it is the focus indicator, and the textarea's quiet
              outline (globals.css) drew a second, smaller box inside this one. */}
          <textarea
            ref={textareaRef}
            rows={1}
            className="block max-h-[240px] min-h-[52px] w-full resize-none bg-transparent px-[16px] pb-[4px] pt-[14px] text-[14px] leading-[1.6] text-text-primary outline-none placeholder:text-text-muted focus-visible:outline-none"
            placeholder={typing ? strings.prompt.steerPlaceholder : strings.prompt.placeholder}
            aria-label={strings.prompt.placeholder}
            onInput={() => {
              grow();
              updateMenu();
            }}
            onKeyDown={handleKeyDown}
            onPaste={paste}
            onBlur={() => window.setTimeout(() => setMenu(null), 150)}
          />

          {images.length === 0 ? null : (
            <div className="flex flex-wrap gap-[6px] px-[12px] pb-[4px]" aria-label={strings.agent.images.attached(images.length)}>
              {images.map((image, index) => (
                <span key={`${image.name}-${index}`} className="group relative inline-flex h-[44px] items-center overflow-hidden rounded-md border border-border-subtle bg-bg-base">
                  {image.data === undefined ? (
                    <span className="px-[8px] font-mono text-[10.5px] text-text-muted">{image.name}</span>
                  ) : (
                    <img src={`data:${image.mediaType};base64,${image.data}`} alt={image.name} className="h-full w-auto max-w-[80px] object-cover" />
                  )}
                  <button
                    type="button"
                    className="absolute right-[2px] top-[2px] grid h-[16px] w-[16px] place-items-center rounded-full bg-bg-raised/90 text-text-secondary opacity-0 transition-opacity group-hover:opacity-100 focus:opacity-100"
                    aria-label={strings.agent.images.remove}
                    title={strings.agent.images.remove}
                    onClick={() => setImages((current) => current.filter((_, other) => other !== index))}
                  >
                    <X size={10} aria-hidden="true" />
                  </button>
                </span>
              ))}
            </div>
          )}

          <div className="prompt-footer flex min-w-0 flex-nowrap items-center gap-[4px] px-[8px] pb-[8px] pt-[2px]">
            <div className="toolbar flex shrink-0 items-center">
              <IconButton icon={Paperclip} label={strings.prompt.toolbar.attach} iconSize={15} onClick={() => attach('file')} />
              <IconButton icon={ImageIcon} label={strings.prompt.toolbar.image} iconSize={15} onClick={() => attach('image')} />
              <VoiceButton
                {...(sessionId === undefined ? {} : { sessionId })}
                onText={(spoken) => {
                  const textarea = textareaRef.current;

                  if (textarea === null) {
                    return;
                  }

                  const present = textarea.value.trimEnd();

                  textarea.value = present === '' ? spoken : `${present} ${spoken}`;
                  textarea.focus();
                  grow();
                }}
              />
            </div>

            <span className="mx-[3px] h-[16px] w-px shrink-0 bg-border-subtle" aria-hidden="true" />

            {/* One mode (0.13): every turn is an agent turn that uses tools only when the request needs them -
                a question is simply answered - the way Claude Code works. The Chat | Agent switch meant
                nothing for the three CLIs and only confused the choice for an API model. */}
            <div className="flex min-w-0 shrink items-center">
              <ModelSelector sessionId={cardSession} />
            </div>
            {/* Which folder this chat works in (0.7.6) - and the way to change it. The pane's own chat, so
                split view's second box never shows the first box's folder (0.10.0). */}
            <div className="flex min-w-0 shrink items-center max-600:hidden">
              <FolderChip sessionId={sessionId} />
            </div>

            {/* Real counts only: an attachment the box does not have is not shown (0.7.x). */}
            {context.files > 0 ? (
              <span className={CHIP}>
                <Hash size={11} aria-hidden="true" />
                <span>{strings.prompt.filesChip(context.files)}</span>
              </span>
            ) : null}

            <ContextChip
              sessionId={sessionId}
              running={running !== null}
              onCompact={() => {
                const textarea = textareaRef.current;

                if (textarea !== null) {
                  textarea.value = '/compact';
                  textarea.focus();
                }
              }}
            />
            <ProcessesChip sessionId={sessionId} />

            <div className="ml-auto flex shrink-0 items-center gap-[6px] pl-[4px]">
              <span className="send-hint hidden font-mono text-[10px] text-text-muted 2xl:inline">
                {typing ? strings.prompt.steerHint : strings.prompt.sendHint}
              </span>

              {typing ? (
                <>
                  <button
                    type="button"
                    className="send-btn grid h-[34px] w-[34px] place-items-center rounded-full bg-accent-fill text-text-on-accent transition-all duration-fast ease-ease hover:bg-accent-hover active:scale-[.94]"
                    title={strings.prompt.steerSend}
                    aria-label={strings.prompt.steerSend}
                    onClick={send}
                  >
                    <ArrowUp size={16} aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    className="stop-btn grid h-[34px] w-[34px] place-items-center rounded-full border border-state-error/60 bg-red-subtle text-state-error transition-all duration-fast ease-ease hover:bg-state-error hover:text-text-on-accent active:scale-[.94]"
                    title={strings.prompt.stopHint}
                    aria-label={strings.prompt.stop}
                    onClick={() => void interruptTurn(running)}
                  >
                    <Square size={11} aria-hidden="true" fill="currentColor" />
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="send-btn grid h-[34px] w-[34px] place-items-center rounded-full bg-accent-fill text-text-on-accent shadow-[0_2px_8px_-2px_var(--accent-glow)] transition-all duration-fast ease-ease hover:bg-accent-hover hover:shadow-[0_3px_12px_var(--accent-glow)] active:scale-[.94]"
                  title={strings.prompt.send}
                  aria-label={strings.prompt.send}
                  onClick={send}
                >
                  <ArrowUp size={16} aria-hidden="true" />
                </button>
              )}
            </div>
          </div>
        </div>
        </div>
      </div>
    </div>
  );
}

/** The media type of a picked image, from its name. */
function mediaTypeOf(path: string): string {
  const lowered = path.toLowerCase();

  if (lowered.endsWith('.jpg') || lowered.endsWith('.jpeg')) {
    return 'image/jpeg';
  }

  if (lowered.endsWith('.gif')) {
    return 'image/gif';
  }

  if (lowered.endsWith('.webp')) {
    return 'image/webp';
  }

  return 'image/png';
}
