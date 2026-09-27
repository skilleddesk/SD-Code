import { AlertTriangle, ChevronDown, ChevronRight, GitBranch, GitCompare, History, Pencil, Redo2, RotateCcw, Undo2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import type { TimelineBranch } from '../../../../protocol/types';
import { strings } from '../../strings';
import { openDiff, redoRewind, refreshAfterTurn, rewindTo } from '../../store/intents';
import { checkpointFileDiff, checkpointFiles, labelCheckpoint, restoreFile, switchBranch, timelineBranches } from '../../store/kernelIntents';
import { useAppStore } from '../../store/store';
import { useSessionsStore } from '../../store/sessions';
import type { CheckpointView } from '../../store/types';
import { BTN, BTN_BLOCK, BTN_SECONDARY, BTN_SM } from '../ui/button';

/**
 * The Time Machine tab - spec section 9.13 / 14, v4's rewind that restores, and 0.12's timeline.
 *
 * Newest checkpoint first. A checkpoint is the folder **before** a change, so every one of them is a place
 * to go back to. A row asks once more before a whole-folder rewind (the first click arms it). 0.12 adds,
 * per checkpoint: a **label** a person gives it (`Before deploy`), **Compare** - every file changed since,
 * each with its own diff and its own **Restore this file** - and the **irreversible** mark: something after
 * it (a push, a deploy, a database command) that a rewind cannot take back, said before the click, not
 * after. Below the list, the chat's **branches**: every rewind keeps the future it left, and switching to
 * one keeps the present as a branch too.
 */
const k = strings.kernel.timeline;

function Row({ entry, armed, onArm }: { entry: CheckpointView; armed: boolean; onArm: () => void }) {
  const [open, setOpen] = useState(false);
  const [files, setFiles] = useState<{ status: string; path: string }[] | null>(null);
  const [diff, setDiff] = useState<{ path: string; text: string } | null>(null);
  const [editing, setEditing] = useState(false);
  const [label, setLabel] = useState(entry.label ?? '');

  const toggle = (): void => {
    const next = !open;

    setOpen(next);

    if (next && files === null) {
      void checkpointFiles(entry.id).then(setFiles);
    }
  };

  return (
    <li
      className={
        'tm-entry mb-[8px] rounded-md border transition-all duration-base ' +
        (armed ? 'border-state-waiting bg-orange-subtle' : 'border-border-subtle bg-bg-raised hover:border-border-strong')
      }
      data-checkpoint={entry.id}
    >
      <div className="flex gap-[10px] p-[10px]">
        {entry.thumbnail === null ? (
          <div className="tm-thumb grid h-[48px] w-[48px] shrink-0 place-items-center rounded-sm border border-border-subtle bg-bg-base text-text-muted" aria-hidden="true">
            <History size={16} />
          </div>
        ) : (
          <img className="tm-thumb h-[48px] w-[72px] shrink-0 rounded-sm object-cover" src={entry.thumbnail} alt="" />
        )}

        <div className="tm-body min-w-0 flex-1">
          <div className="tm-turn font-mono text-[10.5px] text-text-muted">{strings.rightPanel.timeMachine.when(entry.when)}</div>
          {editing ? (
            <input
              className="my-[3px] w-full rounded-sm border border-border-focus bg-bg-input px-[6px] py-[2px] text-[12px]"
              value={label}
              autoFocus
              aria-label={k.labelInput}
              onChange={(event) => setLabel(event.target.value)}
              onBlur={() => setEditing(false)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  void labelCheckpoint(entry.id, label);
                  setEditing(false);
                } else if (event.key === 'Escape') {
                  event.stopPropagation();
                  setEditing(false);
                }
              }}
            />
          ) : (
            <div className="tm-title my-[3px] flex items-center gap-[6px] text-[12.5px] font-medium text-text-primary">
              {entry.label === null || entry.label === undefined || entry.label === '' ? null : (
                <span className="rounded-full bg-accent-subtle px-[7px] py-[1px] text-[10.5px] font-semibold text-accent">{entry.label}</span>
              )}
              <span className="truncate">{entry.title}</span>
            </div>
          )}
          {entry.irreversible === null || entry.irreversible === undefined ? null : (
            <div className="flex items-start gap-[5px] text-[11px] text-state-waiting">
              <AlertTriangle size={11} className="mt-[2px] shrink-0" aria-hidden="true" />
              <span>{k.irreversible(entry.irreversible)}</span>
            </div>
          )}
          <div className="mt-[6px] flex flex-wrap gap-[6px]">
            <button
              type="button"
              className={BTN_SM + ' ' + (armed ? 'border-state-waiting bg-bg-raised text-state-waiting' : BTN_SECONDARY)}
              aria-label={armed ? strings.rightPanel.timeMachine.confirm(entry.title) : strings.rightPanel.timeMachine.choose(entry.title)}
              onClick={onArm}
            >
              <RotateCcw size={11} aria-hidden="true" />
              {armed ? strings.rightPanel.timeMachine.clickAgain : k.rewind}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} aria-expanded={open} onClick={toggle}>
              {open ? <ChevronDown size={11} aria-hidden="true" /> : <ChevronRight size={11} aria-hidden="true" />}
              {k.compare}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => setEditing(true)}>
              <Pencil size={11} aria-hidden="true" />
              {k.label}
            </button>
          </div>
        </div>
      </div>

      {open ? (
        <div className="border-t border-border-subtle px-[10px] py-[8px]">
          {files === null ? <p className="text-[11.5px] text-text-muted">{k.loading}</p> : null}
          {files !== null && files.length === 0 ? <p className="text-[11.5px] text-text-muted">{k.noChanges}</p> : null}
          <ul className="flex flex-col gap-[2px]">
            {(files ?? []).map((file) => (
              <li key={file.path} className="flex items-center gap-[6px] text-[11.5px]">
                <span className="w-[14px] shrink-0 text-center font-mono text-text-muted" title={k.status[file.status as 'A' | 'M' | 'D'] ?? file.status}>
                  {file.status}
                </span>
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left font-mono text-accent hover:underline"
                  dir="ltr"
                  onClick={() => void checkpointFileDiff(entry.id, file.path).then((text) => setDiff(text === null ? null : { path: file.path, text }))}
                >
                  {file.path}
                </button>
                <button
                  type="button"
                  className={BTN_SM + ' ' + BTN_SECONDARY}
                  title={k.restoreFileTitle(file.path)}
                  onClick={() => void restoreFile(entry.id, file.path).then((ok) => (ok ? refreshAfterTurn() : undefined))}
                >
                  <Undo2 size={11} aria-hidden="true" />
                  {k.restoreFile}
                </button>
              </li>
            ))}
          </ul>
          {diff === null ? null : (
            <div className="mt-[8px]">
              <div className="mb-[4px] font-mono text-[10.5px] text-text-muted" dir="ltr">
                {diff.path}
              </div>
              <pre className="max-h-[240px] overflow-auto rounded-sm border border-border-subtle bg-bg-input p-[8px] font-mono text-[10.5px] leading-[1.5]" dir="ltr">
                {diff.text.split('\n').map((line, index) => (
                  <div
                    key={index}
                    className={line.startsWith('+') && !line.startsWith('+++') ? 'bg-diff-addBg text-diff-addText' : line.startsWith('-') && !line.startsWith('---') ? 'bg-diff-removeBg text-diff-removeText' : 'text-text-secondary'}
                  >
                    {line === '' ? ' ' : line}
                  </div>
                ))}
              </pre>
            </div>
          )}
        </div>
      ) : null}
    </li>
  );
}

export function TimeMachineTab() {
  const { activeTab } = useSessionsStore();
  const sessionId = activeTab;
  const all = useAppStore((state) => state.checkpoints);
  const stack = useAppStore((state) => state.rewindStack);
  const [armed, setArmed] = useState<string | null>(null);
  const [branches, setBranches] = useState<TimelineBranch[]>([]);
  const entries = useMemo(() => (sessionId === null ? [] : all.filter((entry) => entry.sessionId === sessionId)), [all, sessionId]);
  const canRedo = sessionId !== null && stack.some((entry) => entry.sessionId === sessionId);

  /* The branches are the daemon's rewind frames; they change whenever the stack does. */
  useEffect(() => {
    if (sessionId === null) {
      setBranches([]);

      return;
    }

    void timelineBranches(sessionId).then(setBranches);
  }, [sessionId, stack.length]);

  if (sessionId === null || (entries.length === 0 && !canRedo && branches.length === 0)) {
    return (
      <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] text-text-muted">
        {strings.rightPanel.timeMachine.empty}
      </div>
    );
  }

  const choose = (id: string, turn: number): void => {
    if (armed !== id) {
      setArmed(id);

      return;
    }

    setArmed(null);
    void rewindTo(sessionId, `turn-${turn}`);
  };

  return (
    <>
      <p className="px-[12px] pt-[12px] text-[11.5px] leading-[1.55] text-text-muted">{strings.rightPanel.timeMachine.hint}</p>

      <ul className="tm-list p-[12px]">
        {entries.map((entry) => (
          <Row key={entry.id} entry={entry} armed={armed === entry.id} onArm={() => choose(entry.id, entry.turn)} />
        ))}
      </ul>

      {branches.length === 0 ? null : (
        <section className="px-[12px] pb-[12px]" aria-label={k.branches}>
          <div className="mb-[6px] flex items-center gap-[6px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">
            <GitBranch size={11} aria-hidden="true" />
            {k.branches}
          </div>
          <ul className="flex flex-col gap-[6px]">
            {branches.map((branch) => (
              <li key={branch.id} className="flex items-center gap-[8px] rounded-md border border-border-subtle bg-bg-raised px-[10px] py-[7px] text-[11.5px]">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-text-primary">{branch.branch ? k.savedPresent : branch.title}</div>
                  <div className="font-mono text-[10px] text-text-muted">{k.branchMeta(branch.checkpoints, branch.turns, branch.pushedAt)}</div>
                </div>
                <button
                  type="button"
                  className={BTN_SM + ' ' + BTN_SECONDARY}
                  disabled={!branch.hasFiles}
                  onClick={() => void switchBranch(sessionId, branch.id).then(() => refreshAfterTurn())}
                >
                  {k.switchTo}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      <div className="tm-footer mt-auto flex flex-col gap-[8px] border-t border-border-subtle p-[12px]">
        {canRedo ? (
          <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_BLOCK} onClick={() => void redoRewind(sessionId)}>
            <Redo2 size={12} aria-hidden="true" />
            {strings.rightPanel.timeMachine.redo}
          </button>
        ) : null}
        <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_BLOCK} onClick={() => void openDiff()}>
          <GitCompare size={12} aria-hidden="true" />
          {strings.rightPanel.timeMachine.compare}
        </button>
      </div>
    </>
  );
}
