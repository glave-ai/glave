import type { GoneFolder, Project, ThreadSummary } from '@glave/client';

import { Refusal, event } from './fake-records';

/**
 * A project whose folder isn't there anymore, "playground", and its thread (`fakeApi`'s
 * `gone`): the server says so on the list, refuses a turn, and lists nothing running, until
 * the person finds the folder or removes the project.
 */
export const PLAYGROUND: Project = {
  id: '01900000-0000-7000-8000-000000000004',
  kind: 'project',
  name: 'playground',
  owner: { kind: 'person', id: '01900000-0000-7000-8000-0000000000aa' },
  created_at: '2026-10-11T10:00:00.000Z',
  updated_at: '2026-10-11T10:00:00.000Z',
};
export const LOST = '01900000-0000-7000-8000-000000000110';
/** What the server says, as glave words it. */
export const GONE_MESSAGE = "This project's folder isn't there anymore.";
/** Where the folder was: a path the person never typed, which the app must never show. */
export const WAS_AT = '/private/tmp/claude-501/scratchpad/playground';

/** The lost thread's history: one exchange, in the folder that went. */
export const LOST_HISTORY = [
  event(1, 'thread.started', {}, LOST),
  event(
    2,
    'workspace.bound',
    { computer: { id: '01900000-0000-7000-8000-0000000000c1', kind: 'local' }, root: WAS_AT },
    LOST,
  ),
  event(3, 'turn.started', {}, LOST),
  event(
    4,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'Sketch a bouncing ball' }] },
    LOST,
  ),
  event(
    5,
    'message.added',
    { role: 'assistant', parts: [{ type: 'text', text: 'Here is a first sketch.' }] },
    LOST,
  ),
  event(6, 'turn.ended', { outcome: 'completed' }, LOST, '2026-10-11T10:00:04.000Z'),
];

/** Where playground stands while the fake runs. */
export interface Gone {
  /** Whether its folder is still missing: until the person finds it. */
  missing: boolean;
  /** Whether the person removed it, its thread archived. */
  removed: boolean;
  /** The folder the person found it at. */
  foundAt: string | null;
}

/** Playground as a fake begins: its folder gone. */
export function folderGone(): Gone {
  return { missing: true, removed: false, foundAt: null };
}

function lost(gone: Gone): ThreadSummary {
  return {
    id: LOST,
    project: PLAYGROUND.id,
    status: 'idle',
    standing: gone.removed ? 'archived' : 'active',
    title: 'Sketch a bouncing ball',
    permissions: 'workspace',
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:04:00.000Z',
    turns: 1,
    tokens: { input: 1, output: 1, cache_read: 0, cache_write: 0, reasoning: 0 },
  };
}

/** Playground as `project/list` shows it: listed until removed, and gone while its folder is. */
export function listedGone(gone: Gone | null): {
  readonly projects: readonly Project[];
  readonly gone: readonly GoneFolder[];
} {
  const listed = gone !== null && !gone.removed;
  return {
    projects: listed ? [PLAYGROUND] : [],
    gone: listed && gone.missing ? [{ project: PLAYGROUND.id, message: GONE_MESSAGE }] : [],
  };
}

/** The refusal of a turn in the lost thread while its folder is gone; else `undefined`. */
export function refusedTurn(params: Record<string, unknown>, gone: Gone | null): unknown {
  return params['thread'] === LOST && gone?.missing === true
    ? new Refusal(-32029, GONE_MESSAGE)
    : undefined;
}

/** The answer to a call about playground or its thread, else `undefined`. */
export function aboutGone(method: string, params: Record<string, unknown>, gone: Gone): unknown {
  const inPlayground = params['project'] === PLAYGROUND.id;
  switch (method) {
    case 'thread/list':
      return inPlayground ? { threads: gone.removed ? [] : [lost(gone)] } : undefined;
    case 'thread/get':
      return params['thread'] === LOST ? { thread: lost(gone) } : undefined;
    case 'command/list':
      return params['thread'] === LOST ? { commands: [] } : undefined;
    case 'project/folder/set':
      if (!inPlayground || !gone.missing) {
        return new Refusal(-32602, 'this project’s folder is still where it was');
      }
      gone.missing = false;
      gone.foundAt = String(params['path']);
      return { project: PLAYGROUND };
    case 'project/remove':
      gone.removed = true;
      return {};
    default:
      return undefined;
  }
}
