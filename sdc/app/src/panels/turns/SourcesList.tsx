import { BookOpen, ExternalLink } from 'lucide-react';

import { openOutside } from '../../lib/external';
import { strings } from '../../strings';
import type { Turn } from './types';

/**
 * The numbered sources under a `/research` answer (0.16.1).
 *
 * Every `[n]` in the answer points at a row here: the page's title, its address and the date it says it
 * was published, so a person can check a claim instead of trusting a small model's summary. Pages the
 * research read come first; pages it only saw in a result list follow, marked as such. A row opens the
 * page in the system browser - only http(s) addresses, the same rule the answer's own links follow.
 */
export function SourcesList({ sources }: { sources: NonNullable<Turn['sources']> }) {
  const words = strings.research.sources;

  return (
    <section
      className="sources-card mb-[10px] overflow-hidden rounded-md border border-border-subtle bg-bg-raised"
      aria-label={words.title(sources.length)}
    >
      <div className="flex items-center gap-[8px] bg-accent-subtle px-[13px] py-[8px] text-[11px]">
        <BookOpen size={13} className="text-accent" aria-hidden="true" />
        <span className="font-semibold uppercase tracking-[.08em] text-accent">{words.title(sources.length)}</span>
      </div>

      <ol className="flex flex-col gap-[6px] px-[13px] py-[9px]">
        {sources.map((source) => {
          const safe = /^https?:\/\//i.test(source.url);
          let host = source.url;

          try {
            host = new URL(source.url).hostname.replace(/^www\./, '');
          } catch {
            /* Not a URL the browser can parse: the address is shown as it came. */
          }

          return (
            <li key={source.n} className={'flex items-start gap-[9px] text-[12.5px] leading-[1.45] ' + (source.read ? '' : 'opacity-75')}>
              <span className="mt-[1px] min-w-[22px] shrink-0 font-mono text-[11px] tabular-nums text-text-muted">[{source.n}]</span>
              <span className="min-w-0 flex-1">
                <button
                  type="button"
                  className="group inline-flex max-w-full items-center gap-[5px] text-left font-medium text-text-primary hover:text-accent disabled:cursor-default disabled:hover:text-text-primary"
                  title={words.open(source.url)}
                  disabled={!safe}
                  onClick={() => {
                    if (safe) {
                      void openOutside(source.url);
                    }
                  }}
                >
                  <span className="truncate">{source.title === '' ? host : source.title}</span>
                  {safe ? <ExternalLink size={11} className="shrink-0 opacity-60 group-hover:opacity-100" aria-hidden="true" /> : null}
                </button>
                <span className="block truncate font-mono text-[11px] text-text-muted">
                  {host}
                  {source.date === null ? '' : ` · ${source.date}`}
                  {source.read ? '' : ` · ${words.seen}`}
                </span>
              </span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}
