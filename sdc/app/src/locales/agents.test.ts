import { describe, expect, it } from 'vitest';

import { PACKS } from './index';
import { strings } from '../strings';

/**
 * Settings → Agents and the Escalate dialog (Phase 3): the tab is in the list, and every word of it exists in Bengali (the owner's language) as
 * well as English. A pack may leave a key out and fall back to English; for these two tables that would put English in front of a person who
 * chose Bengali, so this test refuses it.
 */
const keysOf = (table: Record<string, unknown>): string[] => Object.keys(table).sort();

describe('the Agents tab', () => {
  it('is a Settings tab, after Safety', () => {
    const ids = strings.settings.nav.map((item) => item.id);

    expect(ids).toContain('agents');
    expect(ids.indexOf('agents')).toBe(ids.indexOf('safety') + 1);
  });

  it('has the same strings in Bengali as in English, for the tab and for the Escalate dialog', () => {
    const bn = PACKS.bn as unknown as { agents: Record<string, unknown>; escalate: Record<string, unknown>; profiles: Record<string, unknown>; shipSafe: Record<string, unknown>; telemetry: Record<string, unknown>; brief: Record<string, unknown>; settings: { nav: { id: string }[] } };

    expect(keysOf(bn.agents)).toEqual(keysOf(strings.agents));
    expect(keysOf(bn.escalate)).toEqual(keysOf(strings.escalate));
    expect(keysOf(bn.profiles)).toEqual(keysOf(strings.profiles));
    expect(keysOf(bn.shipSafe)).toEqual(keysOf(strings.shipSafe));
    expect(keysOf(bn.telemetry)).toEqual(keysOf(strings.telemetry));
    expect(keysOf(bn.brief)).toEqual(keysOf(strings.brief));
    expect(bn.settings.nav.map((item) => item.id)).toContain('agents');
  });

  it('says plainly that a pinned model is never switched', () => {
    expect(strings.agents.pinnedExplain).toMatch(/never switches|never switch/i);
    expect(strings.agents.desc).toMatch(/click/i);
  });

  it('shows the cost and what is sent before anything is sent', () => {
    expect(strings.escalate.cost('0.0123')).toContain('0.0123');
    expect(strings.escalate.what).toMatch(/secrets removed/);
    expect(strings.escalate.audit).toContain('sent.jsonl');
  });
});
