import { useMemo, useState } from 'react';
import { Search } from 'lucide-react';

import { Modal } from '../modals/Modal';
import { allSessions } from '../store/reducer';
import { useOverlayStore } from '../store/overlays';
import { usePrefsStore } from '../store/prefs';
import { useAppStore } from '../store/store';
import { toast } from '../store/toast';
import { strings } from '../strings';

/**
 * `#searchBd` - Search everything (spec section 9.4).
 *
 * Three groups, in the prototype's order: the chats on every host, a small file index, and the
 * prompt library. Sessions come from the event log (they are what the daemon knows); files and
 * prompts are static copy, so they live in src/strings.ts.
 *
 * The match is highlighted with `<mark>` rather than a styled `<span>`: it is the semantic element
 * for "this is why the row matched". Clicking a row opens the chat, opens the file, or inserts the
 * prompt into the prompt box - the three verbs the spec gives this overlay.
 */
export function SearchOverlay() {
  const open = useOverlayStore((state) => state.searchOpen);
  const close = useOverlayStore((state) => state.closeSearch);
  const hosts = useAppStore((state) => state.hosts);
  const [query, setQuery] = useState('');

  const needle = query.trim().toLowerCase();

  const matches = useMemo(() => {
    const sessions = allSessions(hosts).filter(
      ({ session }) =>
        needle === '' ||
        session.title.toLowerCase().includes(needle) ||
        session.prompt.toLowerCase().includes(needle),
    );

    return {
      sessions,
      files: strings.search.files.filter((file) => needle === '' || file.includes(needle)),
      prompts: strings.search.prompts.filter(
        (prompt) => needle === '' || prompt.title.toLowerCase().includes(needle),
      ),
    };
  }, [hosts, needle]);

  const empty =
    matches.sessions.length === 0 && matches.files.length === 0 && matches.prompts.length === 0;

  /** Writes a prompt into the prompt box and focuses it, which is what "insert" means here. */
  const insertPrompt = (body: string, title: string): void => {
    close();

    const textarea = document.querySelector<HTMLTextAreaElement>('.prompt-box textarea');

    if (textarea) {
      textarea.value = body;
      textarea.dispatchEvent(new Event('input', { bubbles: true }));
      textarea.focus();
    }

    toast(strings.search.insertedPrompt(title));
  };

  return (
    <Modal open={open} label="Search everything" onClose={close} className="search-overlay">
      <div className="search-head flex items-center gap-[10px] border-b border-border-subtle px-[18px] py-[15px]">
        <Search size={16} aria-hidden="true" className="text-text-muted" />
        <input
          className="search-input min-w-0 flex-1 bg-transparent text-[15px] text-text-primary placeholder:text-text-muted"
          placeholder={strings.search.placeholder}
          aria-label={strings.search.placeholder}
          value={query}
          autoFocus
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault();
              event.stopPropagation();
              close();
            }
          }}
        />
        <span className="kbd inline-flex items-center rounded-[3px] border border-border-default border-b-2 bg-bg-base px-[5px] py-[1px] font-mono text-[9.5px] text-text-secondary">
          Esc
        </span>
      </div>

      <div className="search-body max-h-[52vh] overflow-y-auto px-[6px] pb-[12px] pt-[6px]">
        {empty ? (
          <div className="p-[32px] text-center text-[13px] text-text-muted">
            {strings.search.empty}
          </div>
        ) : null}

        {matches.sessions.length > 0 ? (
          <>
            <GroupLabel text={strings.search.groups.sessions(matches.sessions.length)} />
            {matches.sessions.map(({ host, session }) => (
              <Hit
                key={session.id}
                id={session.id}
                title={<Highlight text={session.title} query={needle} />}
                suffix={`· ${host.name}`}
                snippet={<Highlight text={session.prompt} query={needle} />}
                onPick={() => {
                  close();
                  usePrefsStore.getState().openTab(session.id);
                  usePrefsStore.getState().setActiveHost(host.id);
                }}
              />
            ))}
          </>
        ) : null}

        {matches.files.length > 0 ? (
          <>
            <GroupLabel text={strings.search.groups.files(matches.files.length)} />
            {matches.files.map((file) => (
              <Hit
                key={file}
                title={<Highlight text={file} query={needle} />}
                mono
                onPick={() => {
                  close();
                  toast(strings.search.openedFile(file));
                }}
              />
            ))}
          </>
        ) : null}

        {matches.prompts.length > 0 ? (
          <>
            <GroupLabel text={strings.search.groups.prompts(matches.prompts.length)} />
            {matches.prompts.map((prompt) => (
              <Hit
                key={prompt.title}
                title={<Highlight text={prompt.title} query={needle} />}
                snippet={`${prompt.body.slice(0, 80)}…`}
                onPick={() => insertPrompt(prompt.body, prompt.title)}
              />
            ))}
          </>
        ) : null}
      </div>
    </Modal>
  );
}

/** The uppercase group header of the three lists. */
function GroupLabel({ text }: { text: string }) {
  return (
    <div className="px-[12px] pb-[4px] pt-[10px] text-[10px] uppercase tracking-[0.1em] text-text-muted">
      {text}
    </div>
  );
}

interface HitProps {
  id?: string;
  title: React.ReactNode;
  /** The mono tail of the title line - the host's name. */
  suffix?: string;
  snippet?: React.ReactNode;
  mono?: boolean;
  onPick: () => void;
}

/** One result row. `data-search-hit` is there so the verification harness can find it. */
function Hit({ id, title, suffix, snippet, mono, onPick }: HitProps) {
  return (
    <button
      type="button"
      data-search-hit={id ?? 'row'}
      className="search-hit flex w-full flex-col gap-[4px] rounded-md px-[12px] py-[9px] text-left hover:bg-bg-hover"
      onClick={onPick}
    >
      <span
        className={
          (mono === true ? 'font-mono text-[12.5px]' : 'text-[13px]') +
          ' flex items-center gap-[6px] font-medium text-text-primary'
        }
      >
        {title}
        {suffix === undefined ? null : (
          <span className="font-mono text-[11px] text-text-muted">{suffix}</span>
        )}
      </span>

      {snippet === undefined ? null : (
        <span className="truncate font-mono text-[11.5px] text-text-muted">{snippet}</span>
      )}
    </button>
  );
}

/** The matched substring, wrapped in `<mark>`; everything else is plain text. */
function Highlight({ text, query }: { text: string; query: string }) {
  const index = query === '' ? -1 : text.toLowerCase().indexOf(query);

  if (index < 0) {
    return <>{text}</>;
  }

  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}
