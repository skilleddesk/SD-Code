import { describe, expect, it } from 'vitest';

import { isPushRequest, placeOf, repoNameOf } from './github';

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

describe("each project's own repository (1.0.8)", () => {
  it('is keyed by the machine and the folder, so the same folder name on two servers is two projects', () => {
    expect(placeOf('local', 'C:/work/shop')).toBe('local:C:/work/shop');
    expect(placeOf('vps-1', '/var/www/shop')).not.toBe(placeOf('vps-2', '/var/www/shop'));
    expect(placeOf('local', null)).toBeNull();
    expect(placeOf('local', '')).toBeNull();
  });

  it('names a new repository the way the daemon will', () => {
    expect(repoNameOf('My Shop!')).toBe('My-Shop');
    expect(repoNameOf('site.v2')).toBe('site.v2');
    expect(repoNameOf('  ')).toBe('sdc-project');
  });
});
