import type { PushDevice, PushInfo } from '@glave/client';

import { discovered } from './fake-signin';
import type { Answers } from './fake-records';

/** The browsers a server that pushes keeps for the person, by endpoint, and the kinds they want. */
export interface Pushed {
  readonly devices: Map<string, PushDevice>;
  waiting: boolean;
  finished: boolean;
}

/** A server that pushes, keeping nothing yet and every kind on. */
export function pushing(): Pushed {
  return { devices: new Map(), waiting: true, finished: true };
}

/** What the server says it pushes, the device at `endpoint` marked as the browser asking. */
function offered(pushed: Pushed, endpoint?: string | null): { readonly push: PushInfo } {
  const devices = [...pushed.devices].map(([at, device]) =>
    at === endpoint ? { ...device, current: true } : device,
  );
  return {
    push: {
      kind: 'offered',
      key: 'BPushKeyOfTheFakeServerE2e',
      waiting: pushed.waiting,
      finished: pushed.finished,
      devices,
    },
  };
}

/** `server/discover` saying the server pushes, and `push/*` as a server that pushes answers. */
export const KEPT_PUSH: Answers<Pushed> = {
  'server/discover': () => ({ ...discovered(), push: true }),
  'push/get': (params, pushed) => offered(pushed, params.endpoint),
  'push/subscribe': (params, pushed) => {
    pushed.devices.set(params.endpoint, {
      id: `0190000${String(pushed.devices.size)}-0000-7000-8000-000000000f01`,
      name: params.device,
      added: '2026-11-07T09:00:00.000Z',
    });
    return offered(pushed, params.endpoint);
  },
  'push/remove': (params, pushed) => {
    for (const [at, device] of pushed.devices) {
      if (device.id === params.device) {
        pushed.devices.delete(at);
      }
    }
    return offered(pushed);
  },
  'push/set': (params, pushed) => {
    pushed.waiting = params.waiting;
    pushed.finished = params.finished;
    return offered(pushed);
  },
};
