import { Activity, Database, Square } from 'lucide-react';
import { useEffect, useState } from 'react';

import { strings } from '../../strings';
import { listProcesses, refreshContext, stopProcess, type ProcessRow } from '../../store/agentIntents';
import { useChatModel } from '../../store/chatModel';
import { useAppStore } from '../../store/store';
import { toast } from '../../store/toast';

const CHIP =
  'chip inline-flex h-[26px] items-center gap-[5px] rounded-full px-[9px] font-mono text-[11px] transition-colors duration-fast ease-ease';

const short = (tokens: number): string => (tokens >= 1000 ? `${Math.round(tokens / 1000)}k` : String(tokens));

/**
 * The context meter (0.13): how much of the model's window the next turn of this chat sends. Asked of
 * the daemon when the chat or the model changes and after every turn; kept current during an agent turn
 * by `ContextUpdated`. Amber from 70%, where a click puts `/compact` in the box.
 */
export function ContextChip({ sessionId, running, onCompact }: { sessionId: string | undefined; running: boolean; onCompact: () => void }) {
  const { engine, model, providerId } = useChatModel(sessionId);
  const meter = useAppStore((state) => (sessionId === undefined ? undefined : state.contexts[sessionId]));

  useEffect(() => {
    if (sessionId !== undefined && !running) {
      void refreshContext(sessionId, engine, model, providerId);
    }
  }, [sessionId, engine, model, providerId, running]);

  /* Shown once it is worth a glance (0.13): an empty chat's "ctx 0%" was noise next to Send. */
  if (meter === undefined || (meter.percent < 15 && !meter.compacted)) {
    return null;
  }

  const full = meter.percent >= 70;

  return (
    <button
      type="button"
      className={CHIP + ' ' + (full ? 'text-state-warning hover:bg-bg-hover' : 'text-text-muted hover:bg-bg-hover')}
      title={strings.agent.context.title(short(meter.usedTokens), short(meter.windowTokens), meter.compacted, meter.resumed) + (full ? `\n${strings.agent.context.full}` : '')}
      aria-label={strings.agent.context.chip(meter.percent)}
      onClick={onCompact}
    >
      <Database size={11} aria-hidden="true" />
      <span>{strings.agent.context.chip(meter.percent)}</span>
    </button>
  );
}

/**
 * What an agent left running in this chat (0.13): a dev server, a watcher. Absent when nothing is; a
 * click lists them with a Stop each.
 */
export function ProcessesChip({ sessionId }: { sessionId: string | undefined }) {
  const [rows, setRows] = useState<ProcessRow[]>([]);
  const [open, setOpen] = useState(false);
  const tick = useAppStore((state) => state.turns.length);

  useEffect(() => {
    if (sessionId === undefined) {
      return undefined;
    }

    let alive = true;
    const load = (): void => {
      void listProcesses(sessionId).then((found) => {
        if (alive) {
          setRows(found.filter((row) => row.running));
        }
      });
    };

    load();

    const timer = setInterval(load, 8_000);

    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [sessionId, tick]);

  if (rows.length === 0) {
    return null;
  }

  return (
    <span className="relative">
      <button
        type="button"
        className={CHIP + ' text-state-success hover:bg-bg-hover'}
        aria-expanded={open}
        title={strings.agent.processes.title}
        onClick={() => setOpen((value) => !value)}
      >
        <Activity size={11} aria-hidden="true" />
        <span>{strings.agent.processes.chip(rows.length)}</span>
      </button>
      {open ? (
        <div className="absolute bottom-full left-0 z-30 mb-[6px] w-[min(420px,80vw)] rounded-lg border border-border-default bg-bg-raised p-[8px] shadow-lg">
          <div className="pb-[6px] font-mono text-[10px] uppercase tracking-wide text-text-muted">{strings.agent.processes.title}</div>
          {rows.map((row) => (
            <div key={row.processId} className="flex items-center gap-[8px] py-[4px] text-[12px]">
              <span className="min-w-0 flex-1 truncate font-mono text-text-primary" title={row.command}>
                {row.command}
              </span>
              <span className="shrink-0 text-[10.5px] text-text-muted">{row.place}</span>
              <button
                type="button"
                className="inline-flex shrink-0 items-center gap-[4px] rounded-md border border-state-error px-[7px] py-[2px] text-[11px] text-state-error hover:bg-red-subtle"
                onClick={() =>
                  void stopProcess(row.processId).then((stopped) => {
                    if (stopped) {
                      toast(strings.agent.processes.stopped);
                      setRows((current) => current.filter((other) => other.processId !== row.processId));
                    }
                  })
                }
              >
                <Square size={9} fill="currentColor" aria-hidden="true" />
                {strings.agent.processes.stop}
              </button>
            </div>
          ))}
        </div>
      ) : null}
    </span>
  );
}
