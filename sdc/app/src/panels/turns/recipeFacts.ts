import type { TaskProgressEvent } from '../../../../protocol/types';

/** One review line, taken apart: `place · severity · category · problem · evidence`. */
export interface Finding {
  place: string;
  /** The file part of `place`, or `null` when the place is a web address (it is shown, not opened). */
  file: string | null;
  line: number | null;
  severity: string;
  category: string;
  problem: string;
  evidence: string;
}

const RANK: Record<string, number> = { high: 0, medium: 1, low: 2 };

/** Splits the lines the recipe ranked; the `#` meta lines (`#ran`, `#skipped`, `#limit`) are not findings. Order is kept within a severity. */
export function parseFindings(lines: readonly string[] | undefined): Finding[] {
  const found: Finding[] = [];

  for (const line of lines ?? []) {
    if (line.startsWith('#') || line.trim() === '') {
      continue;
    }

    const [place = '', severity = '', category = '', problem = '', ...rest] = line.split(' · ');
    const isUrl = /^https?:\/\//i.test(place);
    const cut = isUrl ? -1 : place.lastIndexOf(':');
    const row = cut > 0 ? Number(place.slice(cut + 1)) : Number.NaN;
    const hasRow = Number.isInteger(row) && row > 0;

    found.push({
      place,
      file: isUrl || place === '' ? null : hasRow ? place.slice(0, cut) : place,
      line: hasRow ? row : null,
      severity,
      category,
      problem,
      evidence: rest.join(' · '),
    });
  }

  return found
    .map((finding, index) => ({ finding, index }))
    .sort((a, b) => (RANK[a.finding.severity] ?? 3) - (RANK[b.finding.severity] ?? 3) || a.index - b.index)
    .map((entry) => entry.finding);
}

/** The card's own numbers, from the newest progress event. */
export function cardFacts(progress: TaskProgressEvent): { fraction: number; findings: Finding[]; split: [number, number] | null } {
  const findings = parseFindings(progress.findings);
  const hasSplit = (progress.local ?? 0) + (progress.cloud ?? 0) > 0;

  return {
    fraction: progress.total <= 0 ? 0 : Math.min(1, progress.done / progress.total),
    findings,
    split: hasSplit ? [progress.local ?? 0, progress.cloud ?? 0] : null,
  };
}

/** Whether the turn is a recipe the card knows; a plain turn shows no card. */
export function isRecipe(progress: TaskProgressEvent | undefined): progress is TaskProgressEvent {
  return progress !== undefined && progress.recipe.startsWith('R-');
}
