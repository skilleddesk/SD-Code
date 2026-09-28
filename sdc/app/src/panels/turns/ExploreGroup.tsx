import { ChevronRight, FileSearch, Loader } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { summary } from './grouping';
import type { ReadToolData } from './types';

/**
 * Several reads in a row, as one line (0.13): `Explored · Grep add · Read math.js, test.js, package.json`.
 *
 * An agent reads a lot before it changes anything, and five full-height cards for five reads pushed the
 * thing worth reading - what it said, what it changed - off the screen. Claude Code prints "Read 3 files",
 * Codex "Explored"; this is the same idea with the names kept, and a click shows each read with its pill.
 */
export function ExploreGroup({ tools }: { tools: readonly ReadToolData[] }) {
  const [open, setOpen] = useState(false);
  const running = tools.some((tool) => tool.status === 'running');
  const failed = tools.filter((tool) => tool.status === 'failed').length;

  return (
    <div className="explore-group mb-[6px] overflow-hidden rounded-md border border-border-subtle bg-bg-raised" data-explore={tools.length}>
      <div
        className="flex cursor-pointer select-none items-center gap-[10px] px-[13px] py-[8px] text-[12px]"
        role="button"
        tabIndex={0}
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            setOpen((current) => !current);
          }
        }}
      >
        <div className="grid h-[20px] w-[20px] shrink-0 place-items-center rounded-sm border border-border-subtle bg-bg-overlay text-text-secondary">
          <FileSearch size={12} aria-hidden="true" />
        </div>
        <span className="shrink-0 font-mono text-[11.5px] font-semibold text-text-primary">{strings.turns.explored}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-text-secondary" title={summary(tools)}>
          {summary(tools)}
        </span>
        {failed > 0 ? <span className="shrink-0 rounded-sm bg-red-subtle px-[6px] py-[1px] font-mono text-[10px] text-state-error">{strings.turns.exploredFailed(failed)}</span> : null}
        {running ? (
          <Loader size={12} className="animate-spin text-accent" aria-hidden="true" />
        ) : (
          <ChevronRight size={12} aria-hidden="true" className={'text-text-muted transition-transform duration-200 ease-ease ' + (open ? 'rotate-90' : '')} />
        )}
      </div>

      {open ? (
        <div className="border-t border-border-subtle px-[13px] py-[6px]">
          {tools.map((tool, index) => (
            <div key={`${tool.startedAt}-${index}`} className="flex items-center gap-[8px] py-[2px] font-mono text-[11px]">
              <span className="w-[74px] shrink-0 text-text-primary">{tool.name}</span>
              <span className="min-w-0 flex-1 truncate text-text-secondary" title={tool.target}>
                {tool.target}
              </span>
              <span className={'shrink-0 text-[10px] ' + (tool.status === 'failed' ? 'text-state-error' : 'text-text-muted')}>{tool.meta}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
