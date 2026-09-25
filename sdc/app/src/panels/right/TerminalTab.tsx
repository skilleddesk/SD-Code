import { Play, Square, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';

import { strings } from '../../strings';
import { runCommand, runInBackground, stopBackground, terminalSubjectFrom } from '../../store/intents';
import { usePrefsStore } from '../../store/prefs';
import { useSessionsStore } from '../../store/sessions';
import { useTerminalStore, type TerminalEntry } from '../../store/terminal';
import { BTN, BTN_GHOST, BTN_PRIMARY, BTN_SM } from '../ui/button';

/**
 * The Terminal tab - a command surface that runs **here or on the chat's host** (0.7.13).
 *
 * The design law of this repo is that the prototype is the UI's source of truth, and the prototype has
 * six panel tabs and no terminal. This is therefore a *new* surface, drawn from the prototype's own
 * primitives (`panel-tab`, mono type, hairline rows, a 28px button row) rather than invented next to
 * them - the same way 0.7.7 added the FILES section to the sidebar. It sits next to the Console because
 * the two are the panel's two logs: the Console is what the *page* said, this is what a *command* said.
 *
 * Four decisions:
 *
 *   * **it says where the command will run**, above the input, always: `~/app/landing on prod-1`. A
 *     terminal that does not say which machine it is about is the most dangerous surface in a
 *     remote-capable app - `rm -rf build` reads the same on a laptop and on production;
 *   * **it runs through `shell.run`**, so a typed command is a step in the chat: the daemon checkpoints
 *     first, announces the tool call, applies the deny list, and (on a host) runs it *there*;
 *   * **`Run in background` is a second button, not a checkbox**: a dev server must not hold the input,
 *     and its output keeps arriving while you look at another tab (`watchBackground`);
 *   * **a refusal is shown where output goes** - in the entry's stderr slot - because that is where a
 *     terminal puts a reason. It is a sentence from the daemon, not a stack trace.
 *
 * ↑ recalls the last line (a one-line history, which is what a 400px-wide panel can honestly offer), and
 * the log is capped in the store - this is a screen, not an archive. The durable record of a command is
 * the tool-call pair in the session's log, which the Time Machine and the turn stream both read.
 */

/** One run: the line, where it ran, and what it printed. */
function RunRow({ entry, onStop }: { entry: TerminalEntry; onStop: () => void }) {
  const tone =
    entry.state === 'running' ? 'text-accent' : entry.state === 'done' ? 'text-text-muted' : 'text-state-error';
  const outcome =
    entry.state === 'running'
      ? strings.terminal.running
      : entry.state === 'done'
        ? entry.code === null
          ? strings.terminal.ended
          : strings.terminal.exit(entry.code)
        : entry.timedOut
          ? strings.terminal.timedOut
          : strings.terminal.refused;

  return (
    <div className="mb-[6px] rounded-md border-l-2 border-l-border-subtle bg-bg-raised px-[10px] py-[8px]">
      <div className="flex items-start gap-[6px]">
        <span className="shrink-0 font-mono text-[11px] leading-[1.5] text-text-muted">$</span>
        <span className="min-w-0 flex-1 break-words font-mono text-[11px] leading-[1.5] text-text-primary">
          {entry.command}
        </span>

        {entry.state === 'running' ? (
          <button type="button" className={BTN_SM + ' ' + BTN_GHOST} onClick={onStop}>
            <Square size={10} aria-hidden="true" />
            {strings.terminal.stop}
          </button>
        ) : null}
      </div>

      <div className="mt-[3px] flex flex-wrap items-center gap-[8px] font-mono text-[10.5px] text-text-muted">
        <span>{entry.where}</span>
        <span className={tone}>{outcome}</span>
        {entry.state === 'running' || entry.ms === 0 ? null : <span>{strings.terminal.took(entry.ms)}</span>}
      </div>

      {entry.stdout.trim() === '' ? null : (
        <pre className="mt-[6px] max-h-[240px] overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-[1.5] text-text-secondary">
          {entry.stdout.trimEnd()}
        </pre>
      )}

      {entry.stderr.trim() === '' ? null : (
        <pre className="mt-[6px] max-h-[240px] overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-[1.5] text-state-error">
          {entry.stderr.trimEnd()}
        </pre>
      )}
    </div>
  );
}


export function TerminalTab() {
  const { entries, busy, background } = useTerminalStore();
  const [line, setLine] = useState('');
  /*
   * The two subscribes below exist for the *sentence*, not for the click (0.7.13).
   *
   * The panel keeps every tab mounted and hides the inactive ones with a class, so this component is
   * still on screen when you switch to another chat - and a store read would then keep saying `on prod-1`
   * after you moved to a local chat. A terminal that names the wrong machine is the one thing this
   * surface must never do, so the sentence is computed from what this render actually follows.
   */
  const chat = usePrefsStore((state) => state.activeTab);
  const hosts = useSessionsStore((state) => state.hosts);
  const subject = useMemo(() => terminalSubjectFrom(hosts, chat), [hosts, chat]);
  const endRef = useRef<HTMLDivElement | null>(null);

  /* The newest run is the one being read, so the log follows it - the input stays put underneath. */
  useEffect(() => {
    endRef.current?.scrollIntoView({ block: 'nearest' });
  }, [entries.length, background?.ptyId]);

  const submit = (inBackground: boolean): void => {
    const text = line;

    setLine('');

    if (inBackground) {
      void runInBackground(text);
      return;
    }

    void runCommand(text);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (event.key === 'Enter') {
      submit(false);
      return;
    }

    if (event.key === 'ArrowUp') {
      const last = useTerminalStore.getState().history.at(-1);

      if (last !== undefined) {
        event.preventDefault();
        setLine(last);
      }
    }
  };

  const running = background !== null;

  return (
    <>
      {/* Where the next command runs. Mono, because it is a path and a machine name. */}
      <div className="flex flex-wrap items-center gap-[8px] border-b border-border-subtle px-[12px] py-[8px]">
        <span className="font-mono text-[10.5px] text-text-muted">{subject.where}</span>
        {running ? (
          <span className="rounded-full bg-bg-base px-[6px] py-[1px] font-mono text-[9.5px] text-accent">
            {strings.terminal.running}
          </span>
        ) : null}

        <button
          type="button"
          className={BTN_SM + ' ' + BTN_GHOST + ' ml-auto'}
          onClick={() => useTerminalStore.getState().clear()}
          disabled={entries.length === 0}
        >
          <Trash2 size={10} aria-hidden="true" />
          {strings.terminal.clear}
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-[10px]">
        {entries.length === 0 ? (
          <div className="flex h-full items-center justify-center p-[20px] text-center text-[12px] text-text-muted">
            {strings.terminal.empty}
          </div>
        ) : (
          entries.map((entry) => <RunRow key={entry.id} entry={entry} onStop={() => void stopBackground()} />)
        )}

        <div ref={endRef} />
      </div>

      <div className="border-t border-border-subtle p-[10px]">
        <div className="flex items-center gap-[6px]">
          <span className="font-mono text-[11.5px] text-text-muted">$</span>
          <input
            className="h-[28px] min-w-0 flex-1 rounded-md border border-border-default bg-bg-base px-[8px] font-mono text-[11.5px] text-text-primary outline-none focus:border-accent-fill disabled:opacity-40"
            value={line}
            placeholder={strings.terminal.placeholder}
            aria-label={strings.terminal.placeholder}
            disabled={busy}
            onChange={(event) => setLine(event.target.value)}
            onKeyDown={onKeyDown}
          />

          <button
            type="button"
            className={BTN + ' ' + BTN_PRIMARY}
            onClick={() => submit(false)}
            disabled={busy || line.trim() === ''}
          >
            <Play size={11} aria-hidden="true" />
            {strings.terminal.run}
          </button>
        </div>

        <div className="mt-[6px] flex flex-wrap items-center gap-[6px]">
          <button
            type="button"
            className={BTN_SM + ' ' + BTN_GHOST}
            onClick={() => submit(true)}
            disabled={running || line.trim() === ''}
          >
            {strings.terminal.background}
          </button>

          <span className="text-[10.5px] leading-[1.4] text-text-muted">
            {running
              ? strings.terminal.backgroundHint
              : subject.sessionId === null
                ? strings.terminal.hintNoChat
                : strings.terminal.hint}
          </span>
        </div>
      </div>
    </>
  );
}

