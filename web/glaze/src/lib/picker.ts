import type { GlzMenuOption } from '../components/menu-select/menu-select.component';
import { filterOptions } from './options';

/** A group of a picker's list: the pinned options, or all the others. */
export interface GlzPickerGroup {
  readonly kind: 'pinned' | 'all';
  readonly options: readonly GlzMenuOption[];
}

/**
 * The picker's list for `query`: the pinned options that match, in the order they were pinned,
 * then the others that match, in their own order. An empty group is left out.
 */
export function pickerGroups(
  options: readonly GlzMenuOption[],
  pinned: readonly string[],
  query: string,
): readonly GlzPickerGroup[] {
  const matches = filterOptions(options, query);
  const pins = pinned.flatMap((value) => matches.filter((option) => option.value === value));
  const rest = matches.filter((option) => !pinned.includes(option.value));
  const groups: readonly GlzPickerGroup[] = [
    { kind: 'pinned', options: pins },
    { kind: 'all', options: rest },
  ];
  return groups.filter((group) => group.options.length > 0);
}
