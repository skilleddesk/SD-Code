import { AlertTriangle, CheckCircle2, CircleDashed, Inbox, LoaderCircle, Network, Play, Plus, Square, Users } from 'lucide-react';
import type { ReactNode } from 'react';
import { useCallback, useEffect, useState } from 'react';

import { Modal } from '../modals/Modal';
import { Badge } from '../panels/ui/Badge';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { strings } from '../strings';
import {
  agentsBoard,
  inbox,
  listRunners,
  runAutomation,
  startAgent,
  stopAgent,
  type OpsAgent,
  type OpsAutomation,
  type OpsNote,
  type OpsRunner,
} from '../store/kernelIntents';
import { useKernelUi, type OpsTab } from '../store/kernelUi';
import { useSessionsStore } from '../store/sessions';
import { ago, stateOf, whenOf } from './opsView';

/**
 * **The Workbench** (2.0): the three surfaces that are about work SDC starts rather than work you type.
 *
 *   Servers  R4   - the machines this project's turns can go to, and which one is in force
 *   Agents   PAR  - the board: each agent works in its own worktree of the project's own repository, so two
 *                   of them never write over each other
 *   Inbox    AUTO - the automations of `.sdc/automations.toml`, and what they left behind
 *
 * One dialog rather than three rail buttons: they answer one question between them - what is SDC doing without
 * me - and a rail with three more buttons on it is four ways to ask it.
 */
const k = strings.kernel.ops;

const TABS: { id: OpsTab; icon: typeof Network }[] = [
  { id: 'runners', icon: Network },
  { id: 'agents', icon: Users },
  { id: 'inbox', icon: Inbox },
];

const inputClass =
  'rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary focus:border-border-focus';

const CARD = 'rounded-lg border border-border-subtle bg-bg-raised p-[10px]';

function Title({ children }: { children: ReactNode }) {
  return <h3 className="mb-[8px] mt-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{children}</h3>;
}

export function Ops() {
  const open = useKernelUi((state) => state.opsOpen);
  const tab = useKernelUi((state) => state.opsTab);
  const openTab = useKernelUi((state) => state.openOps);
  const close = useKernelUi((state) => state.closeOps);
  return (
    <Modal open={open} label={k.title} onClose={close} center className="flex h-[86vh] w-[min(960px,96vw)] overflow-hidden">
      <nav className="flex w-[180px] shrink-0 flex-col gap-[2px] border-r border-border-subtle bg-bg-raised p-[10px] max-700:w-[60px]" aria-label={k.title}>
        <div className="mb-[8px] px-[8px] text-[13px] font-semibold text-text-primary max-700:hidden">{k.title}</div>
        {TABS.map((item) => {
          const Icon = item.icon;

          return (
            <button
              key={item.id}
              type="button"
              aria-current={item.id === tab}
              className={`flex items-center gap-[10px] rounded-md px-[10px] py-[8px] text-left text-[12.5px] transition-colors duration-fast ${
                item.id === tab ? 'bg-bg-active text-text-primary' : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary'
              }`}
              onClick={() => openTab(item.id)}
            >
              <Icon size={14} />
              <span className="max-700:hidden">{k.tabs[item.id]}</span>
            </button>
          );
        })}
      </nav>

      <div className="flex min-w-0 flex-1 flex-col gap-[10px] overflow-y-auto p-[14px]">
        <h2 className="text-[14px] font-semibold text-text-primary">{k.tabs[tab]}</h2>
        {tab === 'runners' ? <RunnersTab /> : null}
        {tab === 'agents' ? <AgentsTab /> : null}
        {tab === 'inbox' ? <InboxTab /> : null}
      </div>
    </Modal>
  );
}

/** R4: where this project's turns go. The list is `.sdc/runners.toml`, the choice is `.sdc/runner`. */
function RunnersTab() {
  const { activeTab: sessionId } = useSessionsStore();
  const [runners, setRunners] = useState<OpsRunner[] | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    let live = true;

    void (async () => {
      const answer = await listRunners(sessionId ?? '');

      if (!live) return;

      if (!answer) {
        setFailed(k.failed);
        setRunners([]);

        return;
      }

      setRunners(answer.runners);
      setChosen(answer.chosen);
    })();

    return () => {
      live = false;
    };
  }, [sessionId]);

  if (runners === null) return <p className="text-[12px] text-text-muted">{k.loading}</p>;

  return (
    <div className="flex flex-col gap-[8px]">
      {failed ? <p className="text-[12px] text-text-danger">{failed}</p> : null}
      <p className="text-[12px] text-text-secondary">{chosen ? k.runners.chosen(chosen) : k.runners.here}</p>
      {runners.length === 0 ? <p className={`${CARD} text-[12px] text-text-secondary`}>{k.runners.empty}</p> : null}

      {runners.length > 0 ? <Title>{k.tabs.runners}</Title> : null}
      {runners.map((runner) => (
        <div key={runner.name} className={`${CARD} flex items-center justify-between gap-[8px]`}>
          <div className="min-w-0">
            <div className="truncate text-[12px] text-text-primary">{runner.name}</div>
            <div className="truncate text-[11px] text-text-secondary">
              {runner.host} · {runner.dir}
            </div>
          </div>
          {runner.name === chosen ? <Badge tone="success">{k.runners.inForce}</Badge> : null}
        </div>
      ))}
    </div>
  );
}

/** PAR: the board. Start puts an agent in its own worktree and runs the request there; Stop ends its turn. */
function AgentsTab() {
  const { activeTab: sessionId } = useSessionsStore();
  const [agents, setAgents] = useState<OpsAgent[] | null>(null);
  const [request, setRequest] = useState('');
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  const load = useCallback(async () => {
    const answer = await agentsBoard(sessionId ?? '');

    if (answer === null) {
      setFailed(k.failed);

      return;
    }

    setFailed(null);
    setAgents(answer);
  }, [sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  const start = useCallback(async () => {
    const ask = request.trim();

    if (!ask || busy) return;

    setBusy(true);

    const answer = await startAgent(sessionId ?? '', ask);

    setBusy(false);

    if (!answer) {
      setFailed(k.failed);

      return;
    }

    setSaid(k.agents.started(answer.agent.id));
    setRequest('');
    await load();
  }, [busy, load, request, sessionId]);

  const stop = useCallback(
    async (id: string) => {
      setBusy(true);

      const answer = await stopAgent(sessionId ?? '', id);

      setBusy(false);

      if (!answer) {
        setFailed(k.failed);

        return;
      }

      setSaid(k.agents.stopped);
      await load();
    },
    [load, sessionId],
  );

  return (
    <div className="flex flex-col gap-[8px]">
      <div className="flex gap-[6px]">
        <input
          className={`${inputClass} min-w-0 flex-1`}
          placeholder={k.agents.ask}
          value={request}
          onChange={(event) => setRequest(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void start();
          }}
        />
        <button type="button" className={BTN_PRIMARY} disabled={busy || !request.trim()} onClick={() => void start()}>
          <Plus size={13} /> {k.agents.start}
        </button>
      </div>
      <p className="text-[11px] text-text-secondary">{k.agents.needs}</p>
      {said ? <p className="text-[12px] text-text-primary">{said}</p> : null}
      {failed ? <p className="text-[12px] text-text-danger">{failed}</p> : null}

      {agents === null ? <p className="text-[12px] text-text-muted">{k.loading}</p> : null}
      {agents?.length === 0 ? <p className={`${CARD} text-[12px] text-text-secondary`}>{k.agents.empty}</p> : null}

      {agents?.map((agent) => {
        const state = stateOf(agent.state);

        return (
          <div key={agent.id} className={`${CARD} flex flex-col gap-[6px]`}>
            <div className="flex items-center justify-between gap-[8px]">
              <div className="flex min-w-0 items-center gap-[8px]">
                {agent.state === 'running' ? <LoaderCircle size={14} className="shrink-0 animate-spin" /> : null}
                {agent.state === 'done' ? <CheckCircle2 size={14} className="shrink-0" /> : null}
                {agent.state === 'failed' ? <AlertTriangle size={14} className="shrink-0" /> : null}
                {agent.state === 'stopped' ? <CircleDashed size={14} className="shrink-0" /> : null}
                <span className="truncate text-[12px] text-text-primary">{agent.request}</span>
              </div>
              <div className="flex shrink-0 items-center gap-[6px]">
                <Badge tone={state.tone}>{state.label}</Badge>
                {agent.state === 'running' ? (
                  <button type="button" className={BTN_SECONDARY} disabled={busy} onClick={() => void stop(agent.id)}>
                    <Square size={12} /> {k.agents.stop}
                  </button>
                ) : null}
              </div>
            </div>
            <div className="truncate text-[11px] text-text-secondary">{k.agents.worktree(agent.worktree)}</div>
            {agent.note ? <div className="whitespace-pre-wrap text-[11px] text-text-secondary">{agent.note}</div> : null}
          </div>
        );
      })}
    </div>
  );
}

/** AUTO: what the clock runs, and what it left. A note is written in `.sdc/inbox`; its summary is the gist. */
function InboxTab() {
  const { activeTab: sessionId } = useSessionsStore();
  const [automations, setAutomations] = useState<OpsAutomation[] | null>(null);
  const [notes, setNotes] = useState<OpsNote[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  const load = useCallback(async () => {
    const answer = await inbox(sessionId ?? '');

    if (!answer) {
      setFailed(k.failed);
      setAutomations([]);

      return;
    }

    setFailed(null);
    setAutomations(answer.automations);
    setNotes(answer.notes);
  }, [sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = useCallback(
    async (name: string) => {
      setBusy(name);

      const answer = await runAutomation(sessionId ?? '', name);

      setBusy(null);

      if (!answer) {
        setFailed(k.failed);

        return;
      }

      setFailed(null);
      await load();
    },
    [load, sessionId],
  );

  if (automations === null) return <p className="text-[12px] text-text-muted">{k.loading}</p>;

  return (
    <div className="flex flex-col gap-[8px]">
      {failed ? <p className="text-[12px] text-text-danger">{failed}</p> : null}
      {automations.length === 0 ? <p className={`${CARD} text-[12px] text-text-secondary`}>{k.inbox.empty}</p> : null}

      {automations.map((automation) => (
        <div key={automation.name} className={`${CARD} flex items-center justify-between gap-[8px]`}>
          <div className="min-w-0">
            <div className="flex items-center gap-[6px] text-[12px] text-text-primary">
              <span className="truncate">{automation.name}</span>
              {automation.enabled ? null : <Badge tone="warning">{k.inbox.off}</Badge>}
            </div>
            <div className="text-[11px] text-text-secondary">
              {whenOf(automation)} · {k.inbox.lastRun(ago(automation.lastRun))}
            </div>
            <div className="truncate text-[11px] text-text-secondary">{automation.request}</div>
          </div>
          <button
            type="button"
            className={`${BTN} shrink-0`}
            disabled={busy === automation.name || !automation.enabled}
            onClick={() => void run(automation.name)}
          >
            {busy === automation.name ? <LoaderCircle size={12} className="animate-spin" /> : <Play size={12} />}
            {k.inbox.run}
          </button>
        </div>
      ))}

      {notes.length > 0 ? <Title>{k.inbox.title}</Title> : null}
      {notes.map((note) => (
        <div key={`${note.name}-${note.at}`} className={CARD}>
          <div className="flex items-center justify-between gap-[8px] text-[12px] text-text-primary">
            <span className="truncate">{note.name}</span>
            <span className="shrink-0 text-[11px] text-text-secondary">{ago(note.at)}</span>
          </div>
          <div className="mt-[4px] whitespace-pre-wrap text-[11px] text-text-secondary">{note.summary || k.inbox.nothing}</div>
        </div>
      ))}
    </div>
  );
}
