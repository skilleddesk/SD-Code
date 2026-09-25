import { beforeEach, describe, expect, it } from 'vitest';

import { useFilesStore, type OpenFileView } from './files';

/*
 * The open-files strip (v4): a tab per file, the active one is `open`, unsaved text lives in `drafts`
 * and is forgotten with its tab.
 */
function file(path: string, text = 'x'): OpenFileView {
  return { path, name: path.split('/').pop() ?? path, text, sha256: 'a'.repeat(64), bytes: text.length, truncated: false };
}

describe('file tabs', () => {
  beforeEach(() => {
    useFilesStore.getState().reset();
  });

  it('opens each file in its own tab and keeps the newest in front', () => {
    const store = useFilesStore.getState();

    store.setOpen(file('/p/a.ts'));
    store.setOpen(file('/p/b.ts'));

    const state = useFilesStore.getState();

    expect(state.tabs.map((tab) => tab.path)).toEqual(['/p/a.ts', '/p/b.ts']);
    expect(state.open?.path).toBe('/p/b.ts');
  });

  it('re-reading an open file replaces its tab instead of adding one', () => {
    const store = useFilesStore.getState();

    store.setOpen(file('/p/a.ts', 'old'));
    store.setOpen(file('/p/a.ts', 'new'));

    expect(useFilesStore.getState().tabs).toHaveLength(1);
    expect(useFilesStore.getState().open?.text).toBe('new');
  });

  it('closing the front tab brings the one to its left forward, and forgets its draft', () => {
    const store = useFilesStore.getState();

    store.setOpen(file('/p/a.ts'));
    store.setOpen(file('/p/b.ts'));
    store.setOpen(file('/p/c.ts'));
    store.activate('/p/b.ts');
    store.setDraft('/p/b.ts', 'unsaved');
    store.closeTab('/p/b.ts');

    const state = useFilesStore.getState();

    expect(state.tabs.map((tab) => tab.path)).toEqual(['/p/a.ts', '/p/c.ts']);
    expect(state.open?.path).toBe('/p/a.ts');
    expect(state.drafts).toEqual({});
  });

  it('closing a background tab leaves the front one alone', () => {
    const store = useFilesStore.getState();

    store.setOpen(file('/p/a.ts'));
    store.setOpen(file('/p/b.ts'));
    store.closeTab('/p/a.ts');

    expect(useFilesStore.getState().open?.path).toBe('/p/b.ts');
  });

  it('setOpen(null) closes the front tab, and the last one leaves nothing open', () => {
    const store = useFilesStore.getState();

    store.setOpen(file('/p/a.ts'));
    store.setOpen(null);

    expect(useFilesStore.getState().open).toBeNull();
    expect(useFilesStore.getState().tabs).toEqual([]);
  });

  it('a draft that matches the disk again is dropped', () => {
    const store = useFilesStore.getState();

    store.setOpen(file('/p/a.ts'));
    store.setDraft('/p/a.ts', 'changed');
    expect(useFilesStore.getState().drafts).toEqual({ '/p/a.ts': 'changed' });

    store.setDraft('/p/a.ts', null);
    expect(useFilesStore.getState().drafts).toEqual({});
  });

  it('a new folder forgets every tab of the old one', () => {
    const store = useFilesStore.getState();

    store.setRoot('/p');
    store.setOpen(file('/p/a.ts'));
    store.setRoot('/q');

    expect(useFilesStore.getState().tabs).toEqual([]);
    expect(useFilesStore.getState().open).toBeNull();
  });
});
