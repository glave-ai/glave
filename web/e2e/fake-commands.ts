import type { RunningCommand } from '@glave/client';

import { Refusal, bound, call, done, event } from './fake-records';

/**
 * A chat that keeps the docs site running in the background, and what it runs (`fakeApi`'s
 * `commands`).
 */
export const SERVING = '01900000-0000-7000-8000-000000000108';
export const DOCS_SERVER = 'npm run docs -- --port 4321 --host 127.0.0.1 --open --watch';
export const DOCS_COMMAND = '01900000-0000-7000-8000-0000000000d1';
/** When the docs site started, in its thread's turn. */
export const DOCS_STARTED = '2026-10-11T10:00:05.000Z';

/** The serving thread's history: it started the docs site, which still runs. */
export const SERVING_HISTORY = [
  event(1, 'thread.started', {}, SERVING),
  bound(2, SERVING),
  event(3, 'turn.started', {}, SERVING),
  event(
    4,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'Start the docs site so I can look at it' }] },
    SERVING,
  ),
  call(
    5,
    'call_1',
    'bash',
    { command: DOCS_SERVER, background: true },
    `bash: ${DOCS_SERVER}`,
    SERVING,
  ),
  done(6, 'call_1', 'Running in the background as command 1.', SERVING),
  event(
    7,
    'command.started',
    { command: DOCS_COMMAND, call_id: 'call_1', script: DOCS_SERVER },
    SERVING,
    DOCS_STARTED,
  ),
  event(
    8,
    'message.added',
    {
      role: 'assistant',
      parts: [{ type: 'text', text: 'The docs site is running at http://127.0.0.1:4321.' }],
    },
    SERVING,
  ),
  event(9, 'turn.ended', { outcome: 'completed' }, SERVING, '2026-10-11T10:00:06.000Z'),
];

/** Whether the docs site still runs, while the fake runs: until the person stops it. */
export interface Commands {
  running: boolean;
}

/** The docs site, as `command/list` lists it. */
const DOCS: RunningCommand = {
  id: DOCS_COMMAND,
  command: DOCS_SERVER,
  started_at: DOCS_STARTED,
  summary: 'Still running: the agent started it, and it runs until it ends or someone stops it.',
};

/**
 * The answer to a call about a thread's commands in the background, else `undefined`: only the
 * serving thread has one, while it runs.
 */
export function background(
  method: string,
  params: Record<string, unknown>,
  commands: Commands | null,
): unknown {
  const running = params['thread'] === SERVING && commands?.running === true ? commands : null;
  switch (method) {
    case 'command/list':
      return { commands: running === null ? [] : [DOCS] };
    case 'command/stop':
      if (running === null || params['command'] !== DOCS_COMMAND) {
        return new Refusal(-32027, 'that command is not running in this thread');
      }
      running.running = false;
      return {};
    default:
      return undefined;
  }
}
