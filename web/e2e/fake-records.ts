import type { RawEvent, WorkspaceBound } from '@glave/client';

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

/** A step in a line, as the server says a call of glave's own tools: `bash: cargo test`. */
function stepOf(tool: string, json: Record<string, unknown>): string {
  const field = (name: string): string => {
    const value = json[name];
    return typeof value === 'string' ? value : '?';
  };
  switch (tool) {
    case 'bash':
      return `bash: ${field('command')}`;
    case 'write':
      return `write ${field('path')} (${String(field('content').replace(/\n$/u, '').split('\n').length)} lines)`;
    case 'search':
      return `search ${field('pattern')}`;
    case 'web_fetch':
      return `web_fetch ${field('url')}`;
    default:
      return `${tool} ${field('path')}`;
  }
}

/** A tool call, with the line the server records for its step, `summary`. */
export function call(
  seq: number,
  callId: string,
  tool: string,
  json: Record<string, unknown>,
  id = THREAD,
  summary = stepOf(tool, json),
): RawEvent {
  return event(
    seq,
    'tool.call.requested',
    { call_id: callId, tool, arguments: { json }, summary },
    id,
  );
}

export function done(seq: number, callId: string, text: string, id = THREAD): RawEvent {
  return event(
    seq,
    'tool.call.completed',
    { call_id: callId, is_error: false, output: [{ type: 'text', text }] },
    id,
  );
}

/** Where the fake threads work: the project's folder, on the computer glave runs on. */
export const WORKSPACE: WorkspaceBound = {
  workspace: '01900000-0000-7000-8000-0000000000d1',
  computer: { id: '01900000-0000-7000-8000-0000000000c1', kind: 'local' },
  opened: { kind: 'folder', path: '/work/glave' },
  root: '/work/glave',
};

/** Where a chat works: a scratch folder of its own. */
export const SCRATCH: WorkspaceBound = {
  ...WORKSPACE,
  opened: { kind: 'scratch', path: '/home/.glave/scratch/chat' },
  root: '/home/.glave/scratch/chat',
};

/** The thread's workspace, as its summary says it too. */
export function bound(seq: number, id = THREAD): RawEvent {
  return event(seq, 'workspace.bound', { ...WORKSPACE }, id);
}

/** What a server refuses a call with. */
export class Refusal {
  constructor(
    readonly code: number,
    readonly message: string,
  ) {}
}
