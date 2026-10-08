import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { KeyRound, Power, RotateCcw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { sdcpCall } from '../../lib/sdcp';
import { isSdcpError } from '../../lib/transport';
import { strings } from '../../strings';
import type { TerminalSubject } from '../../store/intents';
import { useOverlayStore } from '../../store/overlays';
import { useAppStore } from '../../store/store';
import { BTN_GHOST, BTN_SM } from '../ui/button';

/**
 * The Terminal tab's shell (0.11.7) - a real, interactive terminal for the chat's machine.
 *
 * The report: *"aikhane terminal nai, so ki vabe terminal a kaj korbe"*. The tab ran one command at a
 * time through `shell.run`, which is right for a step the conversation should record and wrong for
 * everything else a terminal is for: a prompt, a `cd` that stays, `top`, `nano`, a REPL, Ctrl+C.
 *
 * How it works, end to end:
 *
 *   * `pty.open { shell: true }` - on a host the daemon starts `ssh -tt` through the **same signed-in
 *     connection** every other call uses, so the host allocates a real terminal and nothing is asked
 *     again (`tty: true`); here it starts the platform's shell on pipes (`tty: false`), and this view
 *     does the line editing a terminal driver would;
 *   * `pty.output { since }` - the raw bytes from a cursor, escape sequences included, written straight
 *     into xterm.js. Polled: every 30 ms while bytes are arriving, backing off to 250 ms when quiet;
 *   * `pty.write` - keystrokes, in order, coalesced while a write is in flight so fast typing and a paste
 *     are one call rather than a queue of them.
 *
 * It opens by itself the first time it is actually on screen (the panel keeps hidden tabs mounted, so
 * "mounted" is not "seen") - a person who clicks Terminal wants a prompt, not a button.
 */
export function ShellView({ subject }: { subject: TerminalSubject }) {
  const box = useRef<HTMLDivElement | null>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  /** The live shell: its id, whether the far side is a real terminal, and where it runs. */
  const shell = useRef<{ ptyId: string; tty: boolean } | null>(null);
  const [where, setWhere] = useState<string | null>(null);
  const [state, setState] = useState<'idle' | 'opening' | 'running' | 'exited'>('idle');
  /* The host refused because nobody is signed in (0.11.8): the header offers the sign-in card. */
  const [signedOut, setSignedOut] = useState(false);
  /* The subject changes with the active chat; the shell is opened with the one on screen at that moment. */
  const subjectRef = useRef(subject);

  subjectRef.current = subject;

  /* Line mode (a shell on pipes): what has been typed since the last Enter. */
  const line = useRef('');
  /* Keystrokes not yet sent, and whether a write is in flight. */
  const outbox = useRef('');
  const sending = useRef(false);
  /* Bumped on every open, so a poll loop left over from a previous shell stops. */
  const generation = useRef(0);

  const send = (data: string): void => {
    const current = shell.current;

    if (current === null) {
      return;
    }

    outbox.current += data;

    if (sending.current) {
      return;
    }

    sending.current = true;

    const flush = (): void => {
      const chunk = outbox.current;

      outbox.current = '';

      if (chunk === '' || shell.current === null) {
        sending.current = false;
        return;
      }

      void sdcpCall('pty.write', { ptyId: current.ptyId, data: chunk }).then(flush, () => {
        sending.current = false;
      });
    };

    flush();
  };

  const poll = async (ptyId: string, mine: number): Promise<void> => {
    let since = 0;
    let delay = 30;

    while (generation.current === mine) {
      let answer;

      try {
        answer = await sdcpCall('pty.output', { ptyId, since });
      } catch {
        break;
      }

      if (generation.current !== mine) {
        return;
      }

      const data = answer.data ?? '';

      if (data.includes('(keyboard-interactive)')) {
        setSignedOut(true);
      }

      if (data !== '') {
        term.current?.write(shell.current?.tty === false ? data.replace(/\r?\n/g, '\r\n') : data);
      }

      since = answer.next ?? since;

      if (answer.state !== 'running' && data === '') {
        break;
      }

      delay = data === '' ? Math.min(delay * 1.5, 250) : 30;
      await new Promise((resolve) => globalThis.setTimeout(resolve, delay));
    }

    if (generation.current === mine) {
      shell.current = null;
      setState('exited');
      term.current?.write(`\r\n\x1b[2m${strings.terminal.shell.ended}\x1b[0m\r\n`);
    }
  };

  const open = async (): Promise<void> => {
    const view = term.current;

    if (view === null) {
      return;
    }

    const mine = ++generation.current;
    const target = subjectRef.current;

    if (shell.current !== null) {
      void sdcpCall('pty.close', { ptyId: shell.current.ptyId }).catch(() => undefined);
      shell.current = null;
    }

    line.current = '';
    outbox.current = '';
    setSignedOut(false);
    view.reset();
    setState('opening');
    setWhere(target.where);
    view.write(`\x1b[2m${strings.terminal.shell.opening(target.where)}\x1b[0m\r\n`);

    try {
      const answer = await sdcpCall('pty.open', {
        shell: true,
        cols: view.cols,
        rows: view.rows,
        ...(target.root === null ? {} : { cwd: target.root }),
        ...(target.sessionId === null ? {} : { sessionId: target.sessionId }),
        ...(target.hostId === undefined ? {} : { hostId: target.hostId }),
      });

      if (generation.current !== mine) {
        void sdcpCall('pty.close', { ptyId: answer.ptyId }).catch(() => undefined);
        return;
      }

      shell.current = { ptyId: answer.ptyId, tty: answer.tty };
      setState('running');
      view.focus();

      /* A shell on pipes prints no prompt of its own; say where the next line runs instead. */
      if (!answer.tty) {
        view.write(`\x1b[2m${strings.terminal.shell.lineMode}\x1b[0m\r\n`);
      }

      void poll(answer.ptyId, mine);
    } catch (error) {
      if (generation.current !== mine) {
        return;
      }

      setState('exited');
      view.write(`\x1b[31m${isSdcpError(error) ? error.message : strings.terminal.shell.failed}\x1b[0m\r\n`);
    }
  };

  const close = (): void => {
    generation.current += 1;

    if (shell.current !== null) {
      void sdcpCall('pty.close', { ptyId: shell.current.ptyId }).catch(() => undefined);
      shell.current = null;
    }

    setState('exited');
    term.current?.write(`\r\n\x1b[2m${strings.terminal.shell.ended}\x1b[0m\r\n`);
  };

  /* The keyboard: straight through to a real terminal, or edited here for a shell on pipes. */
  const onData = (data: string): void => {
    const current = shell.current;
    const view = term.current;

    if (current === null || view === null) {
      return;
    }

    if (current.tty) {
      send(data);
      return;
    }

    for (const char of data) {
      if (char === '\r' || char === '\n') {
        view.write('\r\n');
        send(`${line.current}\n`);
        line.current = '';
      } else if (char === '\x7f' || char === '\b') {
        if (line.current !== '') {
          line.current = line.current.slice(0, -1);
          view.write('\b \b');
        }
      } else if (char === '\x03') {
        line.current = '';
        view.write('^C\r\n');
      } else if (char >= ' ' || char === '\t') {
        line.current += char;
        view.write(char);
      }
    }
  };

  /* Signed in again from the card: the shell that was refused opens by itself. */
  const hostStatus = useAppStore((state) =>
    subject.hostId === undefined ? undefined : state.hosts.find((host) => host.id === subject.hostId)?.status,
  );

  useEffect(() => {
    if (signedOut && hostStatus === 'connected') {
      void openRef.current();
    }
  }, [signedOut, hostStatus]);

  const onDataRef = useRef(onData);

  onDataRef.current = onData;

  const openRef = useRef(open);

  openRef.current = open;

  useEffect(() => {
    const element = box.current;

    if (element === null) {
      return;
    }

    const styles = getComputedStyle(document.documentElement);
    /* xterm.js draws on its own, so it takes the resolved token values, not `var(--…)`; a token that is missing leaves xterm's default. */
    const token = (name: string): string | undefined => styles.getPropertyValue(name).trim() || undefined;
    const view = new Terminal({
      cursorBlink: true,
      fontFamily: '"Geist Mono Variable", ui-monospace, Menlo, Consolas, monospace',
      fontSize: 12,
      lineHeight: 1.15,
      scrollback: 5000,
      allowProposedApi: false,
      theme: {
        background: token('--bg-base'),
        foreground: token('--text-primary'),
        cursor: token('--accent'),
        selectionBackground: token('--accent-subtle'),
      },
    });
    const fitter = new FitAddon();

    view.loadAddon(fitter);
    view.open(element);
    term.current = view;
    fit.current = fitter;

    const typing = view.onData((data) => onDataRef.current(data));
    let opened = false;

    /* Fit to the panel whenever it has a size - and the first time it does, open the shell. */
    const sized = new ResizeObserver(() => {
      if (element.clientWidth === 0 || element.clientHeight === 0) {
        return;
      }

      try {
        fitter.fit();
      } catch {
        return;
      }

      if (!opened) {
        opened = true;
        void openRef.current();
      }
    });

    sized.observe(element);

    return () => {
      generation.current += 1;
      sized.disconnect();
      typing.dispose();

      if (shell.current !== null) {
        void sdcpCall('pty.close', { ptyId: shell.current.ptyId }).catch(() => undefined);
        shell.current = null;
      }

      view.dispose();
      term.current = null;
    };
  }, []);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-[8px] border-b border-border-subtle px-[12px] py-[6px]">
        <span
          className={
            'h-[7px] w-[7px] shrink-0 rounded-full ' +
            (state === 'running' ? 'bg-state-success' : state === 'opening' ? 'bg-state-waiting' : 'bg-text-muted')
          }
          aria-hidden="true"
        />
        <span className="min-w-0 flex-1 truncate font-mono text-[10.5px] text-text-muted">{where ?? subject.where}</span>

        {where !== null && where !== subject.where ? (
          <span className="font-mono text-[10px] text-state-warning">{strings.terminal.shell.otherChat}</span>
        ) : null}

        <button
          type="button"
          className={BTN_SM + ' ' + BTN_GHOST}
          onClick={() => void open()}
          title={strings.terminal.shell.restartHint}
        >
          <RotateCcw size={10} aria-hidden="true" />
          {state === 'running' ? strings.terminal.shell.restart : strings.terminal.shell.open}
        </button>

        {signedOut && subject.hostId !== undefined ? (
          <button
            type="button"
            className={BTN_SM + ' ' + BTN_GHOST + ' text-accent'}
            onClick={() => useOverlayStore.getState().openAddHost(subject.hostId)}
          >
            <KeyRound size={10} aria-hidden="true" />
            {strings.terminal.shell.signIn}
          </button>
        ) : null}

        {state === 'running' ? (
          <button type="button" className={BTN_SM + ' ' + BTN_GHOST} onClick={close}>
            <Power size={10} aria-hidden="true" />
            {strings.terminal.shell.close}
          </button>
        ) : null}
      </div>

      <div className="min-h-0 flex-1 bg-bg-base px-[6px] pt-[4px]" ref={box} data-shell-state={state} />
    </div>
  );
}
