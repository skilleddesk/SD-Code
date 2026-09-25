import { Bot, MessageSquare } from 'lucide-react';

import { strings } from '../../strings';
import { useModelStore, type ComposeMode } from '../../store/model';

/**
 * Chat | Agent (v4, docs/ROADMAP-v4.md §3 #1) - the one switch between asking and getting it done.
 *
 * `Agent` hands the prompt to an agent: the CLIs are agents already, and an API or local model runs
 * inside the daemon's own loop (read, edit, run, check, until the task is finished). `Chat` is a
 * question and an answer. The engine behind it is not the person's problem, so the switch says what
 * each mode *does*, and the tooltip says how much the agent may do on its own at the current mode.
 */
const OPTIONS: readonly { id: ComposeMode; icon: typeof Bot }[] = [
  { id: 'chat', icon: MessageSquare },
  { id: 'agent', icon: Bot },
];

export function ComposeSwitch() {
  const compose = useModelStore((state) => state.compose);
  const setCompose = useModelStore((state) => state.setCompose);

  return (
    <div
      className="compose-switch inline-flex h-[28px] overflow-hidden rounded-md border border-border-default bg-bg-raised"
      role="radiogroup"
      aria-label={strings.prompt.compose.label}
    >
      {OPTIONS.map(({ id, icon: Icon }) => {
        const on = compose === id;

        return (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={on}
            title={strings.prompt.compose.hint[id]}
            className={
              'flex items-center gap-[5px] px-[10px] text-[11.5px] font-medium transition-colors duration-fast focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-border-focus ' +
              (on ? 'bg-accent-fill text-text-on-accent' : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
            }
            onClick={() => setCompose(id)}
          >
            <Icon size={12} aria-hidden="true" />
            {strings.prompt.compose[id]}
          </button>
        );
      })}
    </div>
  );
}
