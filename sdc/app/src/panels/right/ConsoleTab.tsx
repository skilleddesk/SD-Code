import { CircleX, TriangleAlert, WandSparkles, type LucideIcon } from 'lucide-react';

import { strings } from '../../strings';
import { tierName, useModelStore } from '../../store/model';
import type { ConsoleEntry, ConsoleLevel } from '../../store/rightPanel';
import { useAppStore } from '../../store/store';
import { useSessionsStore } from '../../store/sessions';
import { fixWithAgent, openFile } from '../../store/intents';
import { useFilesStore } from '../../store/files';
import { baseName, inFolder } from '../../lib/paths';
import { BTN, BTN_BLOCK, BTN_PRIMARY } from '../ui/button';

/**
 * The Console tab - spec section 7.8.
 *
 * One row per distinct logged line, newest problem first, each with a 2px coloured edge: red for an
 * error, amber for a warning, blue for information. The row's second line is the source - `at
 * LoginForm.tsx:42:11` - and the count pill on the right is how many times that same line was
 * logged, which is the difference between "something broke" and "something broke in a loop".
 *
 * Clicking a row jumps to its source line, which is a toast until the editor integration exists.
 * The footer's `Fix with agent` is the primary action of the whole tab: it hands the failure to the
 * agent, and it opens the Permission dialog because that is the surface that asks before an engine
 * touches a file.
 *
 * The entries live in the right-panel store because the tab strip's badge counts them: the badge is
 * the number of distinct errors, which is exactly `consoleErrorCount()`.
 */

const EDGE: Record<ConsoleLevel, string> = {
  error: 'border-l-state-error',
  warn: 'border-l-state-waiting',
  info: 'border-l-accent',
};

const ICON: Record<ConsoleLevel, LucideIcon> = {
  error: CircleX,
  warn: TriangleAlert,
  info: TriangleAlert,
};

const ICON_TONE: Record<ConsoleLevel, string> = {
  error: 'text-state-error',
  warn: 'text-state-waiting',
  info: 'text-accent',
};

/** Opens the file a console line points at, on its line - in the chat's folder. */
function jump(entry: ConsoleEntry): void {
  const root = useFilesStore.getState().root;

  if (root === null || entry.file === '') {
    return;
  }

  void openFile(inFolder(root, entry.file), baseName(entry.file), entry.line);
}

function ConsoleRow({ entry }: { entry: ConsoleEntry }) {
  const Icon = ICON[entry.level];

  return (
    <div
      className={
        'console-item mb-[6px] flex cursor-pointer gap-[10px] rounded-md border-l-2 bg-bg-raised px-[12px] py-[10px] text-[12px] transition-all duration-fast ease-ease hover:translate-x-[2px] hover:bg-bg-hover ' +
        EDGE[entry.level]
      }
      role="button"
      tabIndex={0}
      onClick={() => jump(entry)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          jump(entry);
        }
      }}
    >
      <div className={ICON_TONE[entry.level]}>
        <Icon size={14} />
      </div>

      <div className="console-body min-w-0 flex-1">
        <div className="console-msg break-words font-mono text-[11px] leading-[1.5] text-text-primary">
          {entry.message}
        </div>
        <div className="console-src mt-[5px] font-mono text-[10.5px] text-text-muted">
          {entry.source}
        </div>
      </div>

      {entry.count > 1 ? (
        <div className="console-count self-start rounded-full bg-bg-base px-[6px] py-[1px] font-mono text-[9.5px] font-semibold text-text-muted">
          {entry.count}
        </div>
      ) : null}
    </div>
  );
}

export function ConsoleTab() {
  const entries = useAppStore((state) => state.console);
  const { activeTab } = useSessionsStore();
  const model = useModelStore();

  if (entries.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] text-text-muted">
        {strings.rightPanel.console.empty}
      </div>
    );
  }

  /** Spec sections 15.5 and 14.9: the console's failure becomes the next turn, then a write ask. */
  const fix = (entry: ConsoleEntry): void => {
    if (activeTab === null) {
      return;
    }

    const sessionId = activeTab;

    /* The agent asks before it edits (its own permission gate), so no second question is raised here -
       this used to open a fixed "Delete a file · src/database.js" dialog after every fix. */
    void fixWithAgent({
      sessionId,
      engine: model.engine,
      model: model.model,
      tier: tierName(model.tier),
      title: entry.message,
      explanation: strings.rightPanel.console.fixPrompt,
      file: entry.file,
      line: entry.line,
    });
  };

  return (
    <>
      <div className="console-list p-[10px]">
        {entries.map((entry) => (
          <ConsoleRow key={`${entry.file}:${entry.line}`} entry={entry} />
        ))}
      </div>

      <div className="mt-auto px-[12px] pb-[12px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY + ' ' + BTN_BLOCK}
          onClick={() => {
            const first = entries[0];

            if (first === undefined) {
              return;
            }

            fix(first);
          }}
        >
          <WandSparkles size={12} aria-hidden="true" />
          {strings.rightPanel.console.fixWithAgent}
        </button>
      </div>
    </>
  );
}
