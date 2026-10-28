import type { Grouping } from '@glave/client';

import type { Answers } from './fake-records';

/**
 * How the person's threads are grouped, as the fake server keeps it for them: by project until
 * they choose. One kept for two browsers is the person's in both.
 */
export interface Grouped {
  grouping: Grouping;
}

/** `grouping/get` and `grouping/set`. */
export const KEPT_GROUPING: Answers<Grouped> = {
  'grouping/get': (_, grouped) => ({ grouping: grouped.grouping }),
  'grouping/set': (params, grouped) => {
    grouped.grouping = params.grouping;
    return { grouping: grouped.grouping };
  },
};
