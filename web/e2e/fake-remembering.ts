import type { Answers } from './fake-records';

/**
 * Whether glave remembers for the person, as the fake server keeps it: on until they choose;
 * with `never`, not theirs to choose, and the server's sentence why.
 */
export interface Remembering {
  on: boolean;
  readonly never: string | null;
}

/** `memory/get` and `memory/set`. */
export const KEPT_REMEMBERING: Answers<Remembering> = {
  'memory/get': (_, kept) => ({
    remembering:
      kept.never === null ? { kind: 'chosen', on: kept.on } : { kind: 'never', reason: kept.never },
  }),
  'memory/set': (params, kept) => {
    kept.on = params.on;
    return { remembering: { kind: 'chosen', on: kept.on } };
  },
};
