import { ChefHat } from 'lucide-react';

import type { TaskProgressEvent } from '../../../../protocol/types';
import { baseName, inFolder } from '../../lib/paths';
import { openFile } from '../../store/intents';
import { useFilesStore } from '../../store/files';
import { strings } from '../../strings';
import { cardFacts, type Finding } from './recipeFacts';

/**
 * The recipe card (PADMA P3-6): the chip says which recipe is running, the bar says unit n of m, and a
 * finished review lists what it found, most serious first, with each file a link that opens it at the line.
 * It draws only what the daemon sent - the same style as the plan card above it, no new look.
 */
export function RecipeCard({ progress }: { progress: TaskProgressEvent }) {
  const root = useFilesStore((state) => state.root);

  return <RecipeCardView progress={progress} onOpen={root === null ? null : (finding) => openFinding(root, finding)} />;
}

function openFinding(root: string, finding: Finding): void {
  if (finding.file !== null) {
    void openFile(inFolder(root, finding.file), baseName(finding.file), finding.line ?? undefined);
  }
}

const SEVERITY: Record<string, string> = {
  high: 'bg-red-subtle text-state-error',
  medium: 'bg-orange-subtle text-state-waiting',
  low: 'bg-bg-hover text-text-muted',
};

export function RecipeCardView({ progress, onOpen }: { progress: TaskProgressEvent; onOpen: ((finding: Finding) => void) | null }) {
  const facts = cardFacts(progress);
  const name = strings.turns.recipe.names[progress.recipe] ?? progress.recipe;
  const state = strings.turns.recipe.state[progress.label] ?? progress.label;
  const reviewed = progress.findings !== undefined;

  return (
    <section
      className="recipe-card mb-[12px] overflow-hidden rounded-lg border border-border-subtle bg-bg-raised shadow-sm"
      aria-label={strings.turns.recipe.title}
      data-recipe={progress.recipe}
    >
      <div className="flex items-center gap-[8px] px-[13px] pb-[2px] pt-[9px] text-[11px]">
        <ChefHat size={13} className="text-accent" aria-hidden="true" />
        <span className="recipe-chip rounded-full border border-border-subtle bg-bg-active px-[8px] py-[1px] font-semibold text-text-secondary">{name}</span>
        <span className="text-text-muted">{state}</span>
        <span className="ml-auto font-mono tabular-nums text-text-muted">{strings.turns.recipe.units(progress.done, progress.total)}</span>
      </div>
      <div className="mx-[13px] mt-[6px] h-[3px] overflow-hidden rounded-full bg-bg-active" aria-hidden="true">
        <div className="h-full rounded-full transition-[width] duration-slow ease-ease [background-image:var(--grad-brand)]" style={{ width: `${Math.round(facts.fraction * 100)}%` }} />
      </div>

      <div className="flex flex-wrap items-center gap-x-[12px] gap-y-[2px] px-[13px] pt-[7px] text-[11.5px] text-text-secondary">
        {reviewed ? <span className="recipe-findings font-medium">{strings.turns.recipe.findings(facts.findings.length)}</span> : null}
        {facts.split === null ? null : <span className="recipe-split font-mono text-text-muted">{strings.turns.recipe.split(facts.split[0], facts.split[1])}</span>}
      </div>

      {reviewed ? (
        facts.findings.length === 0 ? (
          <p className="px-[13px] py-[9px] text-[12px] text-text-muted">{strings.turns.recipe.none}</p>
        ) : (
          <ul className="recipe-findings-list mt-[6px] flex flex-col divide-y divide-border-subtle border-t border-border-subtle" aria-label={strings.turns.recipe.findingsTitle}>
            {facts.findings.map((finding, index) => (
              <li key={`${finding.place}:${index}`} className="finding flex flex-col gap-[3px] px-[13px] py-[8px] text-[12.5px] leading-[1.5]" data-severity={finding.severity}>
                <div className="flex flex-wrap items-center gap-[8px]">
                  <span className={'rounded-full px-[7px] py-[1px] text-[10.5px] font-semibold uppercase tracking-[.06em] ' + (SEVERITY[finding.severity] ?? SEVERITY['low'])}>
                    {strings.turns.recipe.severity[finding.severity] ?? finding.severity}
                  </span>
                  {finding.file !== null && onOpen !== null ? (
                    <button type="button" className="finding-link min-w-0 truncate font-mono text-[11.5px] text-accent hover:underline" title={strings.turns.recipe.open(finding.place)} onClick={() => onOpen(finding)}>
                      {finding.place}
                    </button>
                  ) : (
                    <span className="min-w-0 truncate font-mono text-[11.5px] text-text-muted">{finding.place}</span>
                  )}
                  <span className="text-[11px] text-text-faint">{finding.category}</span>
                </div>
                <span className="text-text-primary">{finding.problem}</span>
              </li>
            ))}
          </ul>
        )
      ) : null}
    </section>
  );
}
