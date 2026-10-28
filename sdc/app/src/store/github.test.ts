import { describe, expect, it } from 'vitest';

import { isPushRequest } from './github';

describe('a chat message that only asks to push', () => {
  it('is recognised in English, Banglish and Bengali', () => {
    for (const said of ['push', '/push', 'Push it.', 'push koro', 'push to github', 'github e push koro', 'পুশ করো', 'গিটহাবে পুশ করে দাও!']) {
      expect(isPushRequest(said)).toBe(true);
    }
  });

  it('leaves a longer request to the model', () => {
    for (const said of ['push the button component to the left', 'fix the bug then push', 'what does git push do?', '']) {
      expect(isPushRequest(said)).toBe(false);
    }
  });
});
