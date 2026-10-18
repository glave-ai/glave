import { NAV_LISTS_KEPT, toNavLists, withList } from './nav-lists';

describe('nav lists', () => {
  it('reads what was stored, and nothing from anything else', () => {
    expect(toNavLists({ collapsed: ['p1', 'chats'], byName: ['projects'] })).toEqual({
      collapsed: ['p1', 'chats'],
      byName: ['projects'],
    });
    expect(toNavLists({ collapsed: ['p1', 7, '', null], byName: 'projects' })).toEqual({
      collapsed: ['p1'],
      byName: [],
    });
    for (const stored of [null, 'chats', 3, ['p1']]) {
      expect(toNavLists(stored)).toEqual({ collapsed: [], byName: [] });
    }
  });

  it('adds a list as the latest change, once, and takes one out', () => {
    expect(withList(['p1', 'p2'], 'p1', true)).toEqual(['p2', 'p1']);
    expect(withList(['p1', 'p2'], 'chats', true)).toEqual(['p1', 'p2', 'chats']);
    expect(withList(['p1', 'p2'], 'p1', false)).toEqual(['p2']);
    expect(withList([], 'p1', false)).toEqual([]);
  });

  it('keeps the lists changed latest, at most a few hundred', () => {
    const many = Array.from({ length: NAV_LISTS_KEPT }, (_, index) => `p${String(index)}`);
    const kept = withList(many, 'new', true);
    expect(kept).toHaveLength(NAV_LISTS_KEPT);
    expect(kept[0]).toBe('p1');
    expect(kept.at(-1)).toBe('new');
    expect(toNavLists({ collapsed: [...many, 'more'] }).collapsed).toHaveLength(NAV_LISTS_KEPT);
  });
});
