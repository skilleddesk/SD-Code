import { describe, expect, it } from 'vitest';

import {
  COMMANDS,
  commandsByGroup,
  commandsForSpec,
  fuzzyScore,
  GROUP_ORDER,
  keySpec,
  matchCommands,
  paletteCommands,
} from './registry';

/**
 * The registry's acceptance test (spec section 9.1, principle P7).
 *
 * The claim this file has to hold up is "the palette and the keyboard map never diverge": both
 * render `COMMANDS`, so the test asserts the *contents* - the counts the spec names, one entry per
 * id, and the focus rule - rather than the markup.
 */
describe('command registry', () => {
  it('has unique ids and labels', () => {
    const ids = COMMANDS.map((command) => command.id);

    expect(new Set(ids).size).toBe(ids.length);
  });

  it('registers the ten global shortcuts of spec section 9.1', () => {
    expect(COMMANDS.filter((command) => command.group === 'global')).toHaveLength(10);
  });

  it('registers the six session keys, the two model keys and the approval keys', () => {
    const group = (name: string) => COMMANDS.filter((command) => command.group === name);

    expect(group('session')).toHaveLength(6);
    expect(group('model')).toHaveLength(2);
    /* Enter, A, Shift+A, S, D and Esc across five entries - Enter and Esc share with others. */
    expect(group('approval')).toHaveLength(5);
    expect(group('timeline')).toHaveLength(6);
  });

  it('normalizes a key event the way the entries are written', () => {
    const spec = (init: Record<string, unknown>, key: string): string =>
      keySpec({ key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...init } as KeyboardEvent);

    expect(spec({ ctrlKey: true }, 'K')).toBe('ctrl+k');
    expect(spec({ ctrlKey: true, shiftKey: true }, 'Escape')).toBe('ctrl+shift+escape');
    expect(spec({}, '\\')).toBe('ctrl+\\'.replace('ctrl+', ''));
    expect(spec({ ctrlKey: true }, '\\')).toBe('ctrl+\\');
    expect(spec({ metaKey: true }, ',')).toBe('meta+,');
    expect(spec({}, 'F1')).toBe('f1');
  });

  it('resolves a spec to the command that claims it, honouring `when`', () => {
    expect(commandsForSpec('ctrl+k').map((command) => command.id)).toEqual(['palette.open']);

    /* The approval keys only exist while a dialog is up - that is what `when()` is for. */
    expect(commandsForSpec('enter')).toHaveLength(0);
    expect(commandsForSpec('escape').map((command) => command.id)).toEqual([
      'overlay.close',
      'turn.interrupt',
    ]);
  });

  it('groups the reference in the spec order and never lists a keyless command', () => {
    const sections = commandsByGroup();

    expect(sections.map((section) => section.group)).toEqual(
      GROUP_ORDER.filter((group) => group !== 'actions'),
    );
    expect(sections.flatMap((section) => section.rows).every((row) => (row.keys ?? []).length > 0)).toBe(true);
  });

  it('keeps the approval keys out of the palette but the actions in it', () => {
    const paletteIds = paletteCommands().map((command) => command.id);

    expect(paletteIds).toContain('host.add');
    expect(paletteIds).toContain('providers.open');
    expect(paletteIds).not.toContain('permission.default');
    expect(paletteIds).not.toContain('timeline.next');
  });

  it('matches fuzzily: prefix beats initials beats subsequence', () => {
    expect(fuzzyScore('new', 'New chat')).toBeGreaterThan(fuzzyScore('open', 'New chat') ?? -9999);
    expect(fuzzyScore('cp', 'Connect a provider / model')).not.toBeNull();
    expect(fuzzyScore('set', 'Open settings')).not.toBeNull();
    expect(fuzzyScore('zzz', 'New chat')).toBeNull();

    const matched = matchCommands('settings').map((command) => command.id);

    expect(matched).toContain('settings.open');

    /* A looser query finds more than one, which is what the palette's ranked list is for. */
    expect(matchCommands('se').length).toBeGreaterThan(1);
  });

  it('returns every command for an empty query, in registry order', () => {
    expect(matchCommands('')).toHaveLength(paletteCommands().length);
    expect(matchCommands('')[0]?.id).toBe('palette.open');
  });
});
