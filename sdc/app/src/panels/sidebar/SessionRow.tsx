import { GitFork, Pencil, Trash2 } from 'lucide-react';

import { strings } from '../../strings';
import {
  formatRelativeTime,
  useSessionsStore,
  type Session,
} from '../../store/sessions';
import { SESSION_STATE_CLASS } from '../ui/status';

/**
 * One row of the sidebar's session tree (spec section 7.3).
 *
 * Layout, left to right: the 7px state dot pinned 6px in from the row's left edge, the title, and a
 * right-hand group with the relative age and - when there is something to say - a badge. The row is
 * deliberately quiet: colour comes from the dot, weight from the active state, and everything else
 * is `--text-secondary` until you hover it.
 *
 * The badge is the interesting part, because there are three reasons for one and they are not
 * equally urgent (spec section 7.3):
 *
 *   unread > 0          a blue count - turns you have not looked at yet
 *   blocked             an amber `!` - the engine is waiting on you (spec gap #12)
 *   error               a red `!` - the turn failed
 *
 * Waiting and error share the `!` because the dot next to the title already carries the colour;
 * repeating it in the badge would be the same message twice.
 *
 * Rename and delete slide in from the right on hover, over a gradient that fades from the row's
 * background to `--bg-hover` so they do not collide with the age. Both are `sess-btn`s - 20x20 -
 * and both stop the click so they do not also open the session.
 */
export interface SessionRowProps {
  session: Session;
  active: boolean;
}

export function SessionRow({ session, active }: SessionRowProps) {
  const { openSession, renameSession, deleteSession, forkSession } = useSessionsStore();

  const unread = session.unread > 0;
  const blocked = session.attention !== undefined;
  const showBadge = unread || blocked || session.state === 'waiting' || session.state === 'error';
  const badgeTone = unread ? 'unread' : session.state === 'error' ? 'error' : 'waiting';
  const badgeClass = {
    unread: 'bg-accent-fill text-text-on-accent',
    waiting: 'bg-orange-subtle text-state-waiting',
    error: 'bg-red-subtle text-state-error',
  }[badgeTone];

  const handleRename = (): void => {
    const next = window.prompt(strings.sidebar.renamePrompt, session.title);

    if (next !== null && next.trim() !== '') {
      renameSession(session.id, next.trim());
    }
  };

  const handleDelete = (): void => {
    if (window.confirm(strings.sidebar.deleteConfirm(session.title))) {
      deleteSession(session.id);
    }
  };

  return (
    <div
      className={
        'session-item group relative flex min-h-[32px] min-w-0 cursor-pointer items-center gap-[8px] ' +
        'rounded-md py-[6px] pr-[8px] pl-[26px] transition-colors duration-fast ease-ease ' +
        (active ? 'active bg-bg-active' : 'hover:bg-bg-hover')
      }
      role="button"
      tabIndex={0}
      title={session.title}
      aria-current={active}
      onClick={() => openSession(session.id)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          openSession(session.id);
        }
      }}
    >
      <span
        className={
          'sdot absolute left-[6px] top-1/2 h-[7px] w-[7px] -translate-y-1/2 rounded-full ' +
          SESSION_STATE_CLASS[session.state]
        }
        aria-hidden="true"
      />

      <span
        className={
          'session-title min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[12.5px] ' +
          (active ? 'font-medium text-text-primary' : 'text-text-secondary')
        }
      >
        {session.title}
      </span>

      <div className="session-right flex shrink-0 items-center gap-[4px]">
        <span className="session-time font-mono text-[10px] text-text-muted">
          {formatRelativeTime(session.minutesAgo)}
        </span>
        {showBadge ? (
          <span
            className={
              'session-badge ' +
              badgeTone +
              ' min-w-[16px] rounded-full px-[5px] py-[1px] text-center font-mono text-[9.5px] font-semibold ' +
              badgeClass
            }
          >
            {unread ? session.unread : strings.sidebar.attentionBadge}
          </span>
        ) : null}
      </div>

      <div
        className={
          'session-hover-actions absolute right-0 top-1/2 hidden -translate-y-1/2 gap-[2px] pr-[2px] pl-[16px] group-hover:flex ' +
          (active
            ? '[background:linear-gradient(90deg,transparent,var(--bg-active)_20%)]'
            : '[background:linear-gradient(90deg,transparent,var(--bg-hover)_20%)]')
        }
      >
        <button
          type="button"
          className="sess-btn grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted transition-all duration-fast ease-ease hover:bg-bg-active hover:text-text-primary"
          title={strings.sidebar.actions.fork}
          aria-label={strings.sidebar.actions.fork}
          onClick={(event) => {
            event.stopPropagation();
            forkSession(session.id, session.title);
          }}
        >
          <GitFork size={11} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="sess-btn grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted transition-all duration-fast ease-ease hover:bg-bg-active hover:text-text-primary"
          title={strings.sidebar.actions.rename}
          aria-label={strings.sidebar.actions.rename}
          onClick={(event) => {
            event.stopPropagation();
            handleRename();
          }}
        >
          <Pencil size={11} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="sess-btn grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted transition-all duration-fast ease-ease hover:bg-bg-active hover:text-text-primary"
          title={strings.sidebar.actions.delete}
          aria-label={strings.sidebar.actions.delete}
          onClick={(event) => {
            event.stopPropagation();
            handleDelete();
          }}
        >
          <Trash2 size={11} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
