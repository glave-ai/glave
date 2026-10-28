import type { Answers } from './fake-records';

/** Whether the person's computer is kept awake while glave works: on until they choose. */
export interface Awake {
  on: boolean;
}

/** `awake/get` and `awake/set` on the person's own computer. */
export const KEPT_AWAKE: Answers<Awake> = {
  'awake/get': (_, awake) => ({ awake: { kind: 'chosen', on: awake.on } }),
  'awake/set': (params, awake) => {
    awake.on = params.on;
    return { awake: { kind: 'chosen', on: awake.on } };
  },
};
