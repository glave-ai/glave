import type { AwakeInfo } from '@glave/client';

/** Whether the person's computer is kept awake while glave works: on until they choose. */
export interface Awake {
  on: boolean;
}

/** The answer to `awake/get` or `awake/set` on the person's own computer, else `undefined`. */
export function keptAwake(
  method: string,
  params: Record<string, unknown>,
  awake: Awake,
): { readonly awake: AwakeInfo } | undefined {
  if (method === 'awake/set') {
    awake.on = params['on'] === true;
  } else if (method !== 'awake/get') {
    return undefined;
  }
  return { awake: { kind: 'chosen', on: awake.on } };
}
