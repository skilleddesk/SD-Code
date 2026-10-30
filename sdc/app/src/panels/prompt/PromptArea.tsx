import {
  ArrowUp,
  Square,
  Hash,
  Image as ImageIcon,
  Paperclip,
  X,
} from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from 'react';

import { strings } from '../../strings';
import { pickFiles, type PickKind, type PickedFile } from '../../lib/picker';
import { interruptTurn, sendPrompt, steerTurn, type TurnImage } from '../../store/intents';
import { findFiles, listCommands } from '../../store/agentIntents';
import { ComposerMenu, type MenuItem } from './ComposerMenu';
import { ContextChip, NoticeChip, ProcessesChip } from './ComposerChips';
import { QuestionCard } from './QuestionCard';
import { runSlash, type SlashOutcome } from './slash';
import { useAppStore } from '../../store/store';
import { useModelStore } from '../../store/model';
import { draftOf, EMPTY_DRAFT, useDraftsStore } from '../../store/drafts';
import { toast } from '../../store/toast';
import { IconButton } from '../ui/IconButton';
import { FolderChip } from './FolderChip';
import { GitHubButton } from './GitHubButton';
import { ModelSelector } from './ModelSelector';
import { QueuedChips } from './QueuedChips';
import { IntentCard } from '../../kernel/IntentCard';
import { ResearchCard } from './ResearchCard';
import { VoiceButton } from '../../kernel/VoiceButton';
import { usePrefsStore } from '../../store/prefs';
import { warmProvider } from '../../store/warm';

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
  /* 1.0.8: everything this box holds is its own chat's (`store/drafts.ts`) - never another chat's. */
  const draftKey = sessionId ?? '';
  const text = useDraftsStore((state) => state.drafts[draftKey]?.text ?? '');
  const images = useDraftsStore((state) => state.drafts[draftKey]?.images ?? EMPTY_DRAFT.images);
  const attached = useDraftsStore((state) => state.drafts[draftKey]?.attached ?? EMPTY_DRAFT.attached);
  const setText = (value: string): void => useDraftsStore.getState().setText(draftKey, value);
  const setImages = (change: (current: readonly TurnImage[]) => readonly TurnImage[]): void =>
    useDraftsStore.getState().setImages(draftKey, change(draftOf(draftKey).images));
  const setAttached = (change: (current: readonly PickedFile[]) => readonly PickedFile[]): void =>
    useDraftsStore.getState().setAttached(draftKey, change(draftOf(draftKey).attached));
  /* What the box held when dictation started (1.0, live voice): the spoken words are written after it. */
  const voiceBase = useRef('');
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

  const [menu, setMenu] = useState<MenuState | null>(null);
  const menuRequest = useRef(0);
  /* The chat whose Intent Contract card this box shows (0.12): the pane's own, or the open one. */
  const activeTab = usePrefsStore((state) => state.activeTab);
  const cardSession = sessionId ?? activeTab;
  const draft = useModelStore((state) => state.draft);
  /* A prompt handed over from elsewhere goes to the chat on screen - in split view, the active one only. */
  const takesHandover = sessionId === undefined || sessionId === activeTab;

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

    if (draft === null || textarea === null || !takesHandover) {
      return;
    }

    useDraftsStore.getState().setText(draftKey, draft);
    textarea.focus();
    useModelStore.getState().setDraft(null);
    window.requestAnimationFrame(() => textarea.setSelectionRange(draft.length, draft.length));
  }, [draft, draftKey, takesHandover]);

  /* The box fits what it holds - typed, spoken, handed over, or restored when the chat is opened again. */
  useLayoutEffect(() => {
    grow();
  }, [text]);
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
      setText(`${item.label} `);
      setMenu(null);

      if (IMMEDIATE.has(item.label.slice(1)) && item.badge === undefined) {
        send();

        return;
      }
    } else {
      const caret = textarea.selectionStart ?? textarea.value.length;

      setText(`${textarea.value.slice(0, menu.start)}@${item.detail} ${textarea.value.slice(caret)}`);
      setMenu(null);
    }

    textarea.focus();
    window.requestAnimationFrame(() => textarea.setSelectionRange(textarea.value.length, textarea.value.length));
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
        const present = draftOf(draftKey).text.trimEnd();

        setText(present === '' ? `${references} ` : `${present} ${references} `);
        textarea.focus();
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
    const prompt = draftOf(draftKey).text.trim();

    if (!textarea || prompt === '') {
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
    /* This chat's turn is still running. First the words go **into** it (0.12.5): an agent turn reads
       them between steps and changes course, as Claude Code does. A turn that cannot take them (a CLI
       turn) leaves them queued as the next turn, as before. */
    if (running !== null && sessionId !== undefined) {
      setText('');

      void steerTurn(running, prompt).then((accepted) => {
        if (accepted) {
          toast(strings.prompt.queued.steered);

          return;
        }

        if (!useModelStore.getState().enqueue(sessionId, prompt)) {
          toast(strings.prompt.queued.full);
          /* The words go back into this chat's own box, not whichever box is on screen now. */
          useDraftsStore.getState().setText(sessionId, prompt);

          return;
        }

        toast(strings.prompt.queued.queuedToast);
      });

      return;
    }

    setText('');
    setMenu(null);
    /* The picked paths travelled *inside* `prompt`, so the count is about the next turn and starts
       again at nothing. */
    setAttached(() => []);

    const withImages = draftOf(draftKey).images;

    setImages(() => []);

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
          /* Into this chat's draft, so it is back even if the person moved to another chat meanwhile. */
          useDraftsStore.getState().setText(draftKey, prompt);
          setImages(() => withImages);
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
    <div className="prompt-area relative shrink-0 px-[24px] pb-[16px] pt-[6px] max-600:px-[14px]">
      {/* The conversation fades into the composer instead of ending on a hard edge: a mask on the scroll
          area (.pane-scroll in globals.css), so the glass panel behind both shows through. */}
      <div className="prompt-inner mx-auto max-w-[780px]">
        {cardSession === null ? null : <IntentCard sessionId={cardSession} />}
        {cardSession === null ? null : <QuestionCard sessionId={cardSession} />}
        <ResearchCard
          sessionId={cardSession}
          onCancel={(words) => {
            setText(words);
            textareaRef.current?.focus();
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
            'prompt-box composer-glass rounded-[20px] border transition-[border-color,box-shadow] duration-base ease-ease ' +
            (typing
              ? 'border-accent/40'
              : 'border-border-default hover:border-border-strong')
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
            value={text}
            onChange={(event) => setText(event.target.value)}
            onInput={() => {
              updateMenu();
              /* 0.21: the provider connection opens while the person types. */
              warmProvider(sessionId);
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

          {/*
            The action row (1.0.3), in three places a hand finds without looking:

              [ ✦ Claude Opus 5.5 ▾ ]        [ 📎  🖼  🎤 ]        [ 📁 my-site ]  ( ↑ )
               the model, left               the tools, centre      the folder, then Send

            The report: "the send / new line text on the right looks odd" - the keys are the Send button's
            tooltip now - and "model on the left, attachment, image and microphone in the middle, the file
            on the right, and only the send icon at the end". A three-column grid, so the tools stay in
            the true centre however long the model's name or the folder's is.

            1.0.8: the row follows **the box's own width**, not the window's (container queries in globals.css,
            `.prompt-box`) - a sidebar, the right panel or split view narrow the box while the window stays wide.
            The right column is never narrower than what it holds, so nothing slides under the tools; as the box
            narrows the chips keep only their icon, then the folder and the model keep only theirs.
          */}
          <div className="prompt-footer grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(max-content,1fr)] items-center gap-[8px] px-[10px] pb-[10px] pt-[4px]">
            <div className="composer-model flex min-w-0 items-center justify-start">
              <ModelSelector sessionId={cardSession} />
            </div>

            <div
              className="composer-tools flex shrink-0 items-center gap-[2px] rounded-full border border-border-subtle bg-bg-raised p-[3px] shadow-[inset_0_1px_0_var(--edge-light)]"
              role="toolbar"
              aria-label={strings.prompt.toolbar.label}
            >
              <IconButton icon={Paperclip} label={strings.prompt.toolbar.attach} size={30} iconSize={15} className="!rounded-full" onClick={() => attach('file')} />
              <IconButton icon={ImageIcon} label={strings.prompt.toolbar.image} size={30} iconSize={15} className="!rounded-full" onClick={() => attach('image')} />
              <VoiceButton
                {...(sessionId === undefined ? {} : { sessionId })}
                /* 1.0: the words are written into the box while the person speaks - after what was typed before. */
                /* 1.0.8: the spoken words go into **this chat's** draft - a dictation that ends after the person
                   moved to another chat lands here, never in the box on screen. */
                onStart={() => {
                  voiceBase.current = draftOf(draftKey).text.trimEnd();
                }}
                onLive={(spoken) => {
                  useDraftsStore.getState().setText(draftKey, [voiceBase.current, spoken].filter((part) => part !== '').join(' '));

                  const textarea = textareaRef.current;

                  if (textarea !== null) {
                    window.requestAnimationFrame(() => {
                      textarea.scrollTop = textarea.scrollHeight;
                    });
                  }
                }}
                onText={(spoken) => {
                  useDraftsStore.getState().setText(draftKey, [voiceBase.current, spoken].filter((part) => part !== '').join(' '));
                  textareaRef.current?.focus();
                }}
              />
            </div>

            <div className="composer-right flex min-w-0 items-center justify-end gap-[6px]">
              {/* Real counts only: an attachment the box does not have is not shown (0.7.x). */}
              {context.files > 0 ? (
                <span className={CHIP + ' composer-files'}>
                  <Hash size={11} aria-hidden="true" />
                  <span>{strings.prompt.filesChip(context.files)}</span>
                </span>
              ) : null}

              <ContextChip
                sessionId={sessionId}
                running={running !== null}
                onCompact={() => {
                  setText('/compact');
                  textareaRef.current?.focus();
                }}
              />
              <NoticeChip sessionId={sessionId} />
              <ProcessesChip sessionId={sessionId} />

              {/* Which folder this chat works in (0.7.6) - and the way to change it. The pane's own chat, so
                  split view's second box never shows the first box's folder (0.10.0). */}
              <div className="composer-folder flex min-w-0 items-center">
                <FolderChip sessionId={sessionId} />
              </div>

              {/* 1.0.8: this project's own GitHub repository - one click pushes it (GitHubButton.tsx). */}
              <GitHubButton sessionId={sessionId} />

              {typing ? (
                <>
                  <button
                    type="button"
                    className="send-btn brand-fill grid h-[34px] w-[34px] shrink-0 place-items-center rounded-full bg-accent-fill text-text-on-accent active:scale-[.94]"
                    title={`${strings.prompt.steerSend} · ${strings.prompt.steerHint}`}
                    aria-label={strings.prompt.steerSend}
                    onClick={send}
                  >
                    <ArrowUp size={16} aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    className="stop-btn grid h-[34px] w-[34px] shrink-0 place-items-center rounded-full border border-state-error/60 bg-red-subtle text-state-error transition-all duration-fast ease-ease hover:bg-state-error hover:text-text-on-accent active:scale-[.94]"
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
                  className="send-btn brand-fill grid h-[34px] w-[34px] shrink-0 place-items-center rounded-full bg-accent-fill text-text-on-accent active:scale-[.94]"
                  title={`${strings.prompt.send} · ${strings.prompt.sendHint}`}
                  aria-label={strings.prompt.send}
                  onClick={send}
                >
                  <ArrowUp size={16} strokeWidth={2.4} aria-hidden="true" />
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
