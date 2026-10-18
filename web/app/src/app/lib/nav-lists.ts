/**
 * How the person left the side nav's lists in this browser: those they closed, and those they
 * sorted by name rather than by recent activity, each by its key (`PROJECTS_LIST`, `CHATS_LIST`
 * or a project's id), latest change last.
 */
export interface NavLists {
  readonly collapsed: readonly string[];
  readonly byName: readonly string[];
}

/** The most lists of each kind kept: those changed longest ago are forgotten first. */
export const NAV_LISTS_KEPT = 200;

/** The lists as they were stored, or none closed and none by name when that is unreadable. */
export function toNavLists(value: unknown): NavLists {
  const field = (name: keyof NavLists): readonly string[] => {
    const list: unknown =
      typeof value === 'object' && value !== null ? Reflect.get(value, name) : null;
    return Array.isArray(list)
      ? list
          .filter((each): each is string => typeof each === 'string' && each !== '')
          .slice(-NAV_LISTS_KEPT)
      : [];
  };
  return { collapsed: field('collapsed'), byName: field('byName') };
}

/**
 * `lists` with `list` in it (`on`) or not: added last, as the latest change, at most
 * `NAV_LISTS_KEPT` in all.
 */
export function withList(lists: readonly string[], list: string, on: boolean): readonly string[] {
  const others = lists.filter((each) => each !== list);
  return (on ? [...others, list] : others).slice(-NAV_LISTS_KEPT);
}
