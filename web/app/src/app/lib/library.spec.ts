import { picture, record } from '../testing/fixtures';
import {
  EVERY,
  filtered,
  kindChoices,
  kindLabel,
  madeWhen,
  projectChoices,
  shown,
} from './library';

describe('library', () => {
  const items = [picture('fox', 'p1'), picture('hare', 'chats'), picture('owl', 'p1')];

  it('keeps every item, or those of one kind and one project, in order', () => {
    const all = filtered(items, { kind: EVERY, project: EVERY });
    expect(all).toEqual(items);
    const names = filtered(items, { kind: 'picture', project: 'p1' }).map((each) => each.name);
    expect(names).toEqual(['fox', 'owl']);
    expect(filtered(items, { kind: 'picture', project: 'p9' })).toEqual([]);
  });

  it('offers every kind, and the projects the items are in, by name, the Chats as the app says', () => {
    expect(kindChoices()).toEqual([
      { value: EVERY, label: 'Everything' },
      { value: 'picture', label: 'Pictures' },
    ]);
    expect(kindLabel('picture')).toBe('Pictures');
    const projects = [
      record('p1', 'glave'),
      record('p2', 'empty'),
      // The server's name for it, which the person never sees.
      record('chats', 'Personal', 'personal'),
    ];
    expect(projectChoices(items, projects)).toEqual([
      { value: EVERY, label: 'Every project' },
      { value: 'p1', label: 'glave' },
      { value: 'chats', label: 'Chats' },
    ]);
  });

  it('says when an item was made: today with its time, yesterday, a weekday, else its date', () => {
    const reader = { locale: 'en-US', timeZone: 'UTC' };
    const now = Date.parse('2026-11-16T18:00:00Z');
    expect(madeWhen('2026-11-16T09:12:00Z', now, reader)).toBe('Today, 9:12 AM');
    expect(madeWhen('2026-11-15T23:00:00Z', now, reader)).toBe('Yesterday');
    expect(madeWhen('2026-11-12T10:00:00Z', now, reader)).toBe('Thursday');
    expect(madeWhen('2026-11-02T10:00:00Z', now, reader)).toBe('November 2');
    expect(madeWhen('2025-11-02T10:00:00Z', now, reader)).toBe('November 2, 2025');
    expect(madeWhen('not a time', now, reader)).toBe('');
  });

  it('draws an item with its chat while that chat is there', () => {
    const reader = { locale: 'en-US', timeZone: 'UTC' };
    const now = Date.parse('2026-11-16T18:00:00Z');
    const drawn = shown(picture('fox', 'p1', 't1'), now, reader);
    expect(drawn.when).toBe('Today, 9:12 AM');
    expect(drawn.openLabel).toBe('Open fox');
    expect(drawn.chat).toEqual({ link: '/t/t1', title: 'About fox' });
    expect(shown(picture('fox', 'p1'), now, reader).chat).toBeNull();
  });
});
