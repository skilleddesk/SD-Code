import { FolderOpen } from 'lucide-react';

import { nameOf } from '../../lib/picker';
import { strings } from '../../strings';
import { changeFolder } from '../../store/intents';
import { findSession, useSessionsStore } from '../../store/sessions';

/**
 * `.folder-chip` - which directory this chat works in, and the way to change it (0.7.6).
 *
 * It sits in the prompt toolbar beside the model selector, because "which folder" is the same kind of
 * fact as "which model": a choice the next turn will be made under, and one the person needs to be able
 * to read at a glance. It shows the **last path segment** with the **full path as its tooltip**, which is
 * the only way a narrow toolbar can hold `H:\work\clients\sdc\app` without becoming a scroll bar.
 *
 * Before 0.7.6 there was nothing to show: a chat had no working directory, so the engines were started in
 * whatever folder the daemon had been started in, and the app could not have said where that was even if
 * it had wanted to. The chip is therefore honest in both states - `Working in SDC` when the chat has a
 * folder, `No folder` when it has none - and pressing it opens the same native folder dialog `Open folder`
 * uses, pointing *this chat* somewhere else.
 *
 * It renders nothing when no chat is open: there is no chat whose folder could be shown, and the empty
 * state has the button that matters in that moment.
 */
export function FolderChip() {
  const { hosts, activeTab } = useSessionsStore();
  const found = activeTab === null ? null : findSession(hosts, activeTab);

  if (found === null) {
    return null;
  }

  const { session } = found;
  const root = session.projectRoot ?? null;
  const name = root === null ? null : nameOf(root);

  return (
    <button
      type="button"
      className="folder-chip chip inline-flex h-[28px] max-w-[260px] items-center gap-[5px] rounded-md border border-border-subtle bg-bg-raised px-[8px] py-[3px] font-mono text-[11px] text-text-secondary transition-all duration-fast ease-ease hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
      /* The tooltip is the whole path, which is the part the label had to drop. */
      title={root === null ? strings.folder.change : `${root} · ${strings.folder.change}`}
      aria-label={root === null ? strings.folder.change : `${root} · ${strings.folder.change}`}
      onClick={() => void changeFolder(session.id)}
    >
      <FolderOpen size={10} aria-hidden="true" className="shrink-0" />
      <span className="truncate">
        {name === null ? strings.folder.none : strings.folder.workingIn(name)}
      </span>
    </button>
  );
}
