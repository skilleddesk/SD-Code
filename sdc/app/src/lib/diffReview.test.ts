import { describe, expect, it } from 'vitest';

import { hunkDraft, MAX_QUOTE, parseDiff, selectionDraft } from './diffReview';

const PATCH = [
  'diff --git a/src/a.py b/src/a.py',
  'index 1111111..2222222 100644',
  '--- a/src/a.py',
  '+++ b/src/a.py',
  '@@ -1,3 +1,3 @@ def f():',
  ' keep',
  '-old',
  '+new',
  '@@ -10,2 +10,3 @@',
  ' tail',
  '+extra one',
  '+extra two',
  'diff --git a/img.png b/img.png',
  'new file mode 100644',
  'Binary files /dev/null and b/img.png differ',
  '',
].join('\n');

describe('parseDiff', () => {
  it('reads files, hunks and counts from the patch markers', () => {
    const files = parseDiff(PATCH);

    expect(files.map((file) => file.path)).toEqual(['src/a.py', 'img.png']);
    expect(files[0].hunks.map((hunk) => [hunk.added, hunk.removed])).toEqual([[1, 1], [2, 0]]);
    expect(files[0].meta).toContain('index 1111111..2222222');
    expect(files[0].meta).not.toContain('@@');
  });

  it('keeps each hunk exactly as git printed it, header first', () => {
    const [first, second] = parseDiff(PATCH)[0].hunks;

    expect(first.text).toBe('@@ -1,3 +1,3 @@ def f():\n keep\n-old\n+new');
    expect(second.text.startsWith('@@ -10,2 +10,3 @@')).toBe(true);
    expect(second.text.endsWith('+extra two')).toBe(true);
  });

  it('shows a file with no hunks (binary, mode change) with its own description', () => {
    const image = parseDiff(PATCH)[1];

    expect(image.hunks).toEqual([]);
    expect(image.meta).toContain('Binary files');
  });

  it('answers nothing for an empty or non-diff text', () => {
    expect(parseDiff('')).toEqual([]);
    expect(parseDiff('just some text\n@@ not a hunk')).toEqual([]);
  });

  it('does not count the --- and +++ file lines as changes', () => {
    const [file] = parseDiff('diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n');

    expect(file.hunks[0]).toMatchObject({ added: 1, removed: 1 });
  });
});

describe('prompts that point at code', () => {
  it('quotes a hunk under the question, with its file', () => {
    const draft = hunkDraft('src/a.py', parseDiff(PATCH)[0].hunks[0], 'Why was this changed?');

    expect(draft.startsWith('Why was this changed?\n\nIn src/a.py:\n```diff\n@@')).toBe(true);
    expect(draft.endsWith('```\n')).toBe(true);
  });

  it('references a selection as @path:line and trims trailing space', () => {
    expect(selectionDraft('src/a.py', 'x = 1  \n', 'Make this a constant', 12)).toBe('Make this a constant\n\n@src/a.py:12\n```\nx = 1\n```\n');
    expect(selectionDraft('src/a.py', 'x', 'Q')).toContain('@src/a.py\n');
  });

  it('cuts a very long quote instead of flooding the prompt', () => {
    const draft = selectionDraft('a', 'y'.repeat(MAX_QUOTE * 2), 'Q');

    expect(draft.length).toBeLessThan(MAX_QUOTE + 100);
    expect(draft).toContain('(cut)');
  });
});
