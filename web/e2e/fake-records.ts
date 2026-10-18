import type { RawEvent } from '@glave/client';

/**
 * What the fake server's threads record and how it refuses, as the session API shapes them:
 * shared by fake-api.ts and the fakes beside it. Records are of `THREAD`, the thread most tests
 * open, unless they name another.
 */
export const THREAD = '01900000-0000-7000-8000-000000000101';

export function event(
  seq: number,
  type: string,
  data: RawEvent['data'],
  id = THREAD,
  at = '2026-10-11T10:00:00.000Z',
): RawEvent {
  return {
    glave: 1,
    id: `01900000-0000-7000-8000-${String(seq).padStart(12, '0')}`,
    thread: id,
    seq,
    at,
    type,
    v: 1,
    data,
  };
}

export function call(
  seq: number,
  callId: string,
  tool: string,
  json: Record<string, unknown>,
  id = THREAD,
): RawEvent {
  return event(seq, 'tool.call.requested', { call_id: callId, tool, arguments: { json } }, id);
}

export function done(seq: number, callId: string, text: string, id = THREAD): RawEvent {
  return event(
    seq,
    'tool.call.completed',
    { call_id: callId, is_error: false, output: [{ type: 'text', text }] },
    id,
  );
}

/** The thread's workspace: a folder on the computer glave runs on. */
export function bound(seq: number, id = THREAD): RawEvent {
  const computer = { id: '01900000-0000-7000-8000-0000000000c1', kind: 'local' };
  return event(seq, 'workspace.bound', { computer, root: '/work/glave' }, id);
}

/** What a server refuses a call with. */
export class Refusal {
  constructor(
    readonly code: number,
    readonly message: string,
  ) {}
}
