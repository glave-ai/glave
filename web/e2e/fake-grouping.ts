import type { Grouping } from '@glave/client';

/**
 * How the person's threads are grouped, as the fake server keeps it for them: by project until
 * they choose. One kept for two browsers is the person's in both.
 */
export interface Grouped {
  grouping: Grouping;
}

/** The answer to `grouping/get` or `grouping/set`, else `undefined`. */
export function keptGrouping(
  method: string,
  params: Record<string, unknown>,
  grouped: Grouped,
): { readonly grouping: Grouping } | undefined {
  if (method === 'grouping/set') {
    grouped.grouping = params['grouping'] === 'status' ? 'status' : 'project';
  } else if (method !== 'grouping/get') {
    return undefined;
  }
  return { grouping: grouped.grouping };
}
