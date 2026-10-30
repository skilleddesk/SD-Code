import { beforeEach, describe, expect, it } from 'vitest';

import { draftOf, useDraftsStore } from './drafts';

describe("each chat's own unsent message (1.0.8)", () => {
  beforeEach(() => {
    useDraftsStore.setState({ drafts: {} });
  });

  it('keeps what is typed in one chat out of every other chat', () => {
    useDraftsStore.getState().setText('chat-a', 'half a sentence');
    useDraftsStore.getState().setImages('chat-a', [{ name: 'shot.png', mediaType: 'image/png', data: 'AAAA' }]);

    expect(draftOf('chat-a').text).toBe('half a sentence');
    expect(draftOf('chat-a').images).toHaveLength(1);
    expect(draftOf('chat-b')).toEqual({ text: '', images: [], attached: [] });
  });

  it('finds a chat as it was left when it is opened again', () => {
    useDraftsStore.getState().setText('chat-a', 'first');
    useDraftsStore.getState().setText('chat-b', 'second');
    useDraftsStore.getState().setAttached('chat-b', [{ path: '/srv/site/index.php', name: 'index.php' }]);

    expect(draftOf('chat-a').text).toBe('first');
    expect(draftOf('chat-b').text).toBe('second');
    expect(draftOf('chat-b').attached).toHaveLength(1);
    expect(draftOf('chat-a').attached).toHaveLength(0);
  });

  it('forgets a closed chat', () => {
    useDraftsStore.getState().setText('chat-a', 'gone');
    useDraftsStore.getState().clear('chat-a');

    expect(draftOf('chat-a').text).toBe('');
  });
});
