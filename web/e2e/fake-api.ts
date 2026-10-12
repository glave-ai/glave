import type { Page, Route } from '@playwright/test';

/** What the fake server knows: one space with one thread, and what a turn streams. */
const SPACE = {
  id: '01900000-0000-7000-8000-000000000001',
  kind: 'project',
  name: 'glave',
  owner: { kind: 'person', id: '01900000-0000-7000-8000-0000000000aa' },
  members: [],
  created_at: '2026-10-11T10:00:00.000Z',
  updated_at: '2026-10-11T10:00:00.000Z',
};

const TOKENS = { input: 1200, output: 300, cache_read: 0, cache_write: 0, reasoning: 0 };

function thread(id: string, title: string, status = 'idle'): Record<string, unknown> {
  return {
    id,
    space: SPACE.id,
    status,
    standing: 'active',
    title,
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:05:00.000Z',
    turns: 1,
    tokens: TOKENS,
  };
}

export const THREAD = '01900000-0000-7000-8000-000000000101';
export const NEW_THREAD = '01900000-0000-7000-8000-000000000102';

function event(
  seq: number,
  type: string,
  data: Record<string, unknown>,
  id = THREAD,
): Record<string, unknown> {
  return {
    glave: 1,
    id: `01900000-0000-7000-8000-${String(seq).padStart(12, '0')}`,
    thread: id,
    seq,
    at: '2026-10-11T10:00:00.000Z',
    type,
    v: 1,
    data,
  };
}

const HISTORY = [
  event(1, 'thread.started', {}),
  event(2, 'message.added', {
    role: 'user',
    parts: [{ type: 'text', text: 'Make the tests pass' }],
  }),
  event(3, 'tool.call.requested', {
    call_id: 'call_1',
    tool: 'bash',
    arguments: { json: { command: 'cargo test' } },
  }),
  event(4, 'tool.call.completed', {
    call_id: 'call_1',
    is_error: false,
    output: [{ type: 'text', text: 'ok' }],
  }),
  event(5, 'message.added', {
    role: 'assistant',
    parts: [{ type: 'text', text: 'They pass now.' }],
  }),
];

/** Answers the session API's calls the app makes, as JSON-RPC over POST /. */
export async function fakeApi(page: Page): Promise<{ readonly calls: readonly string[] }> {
  const calls: string[] = [];
  await page.route('**/*', async (route: Route) => {
    const request = route.request();
    if (request.method() !== 'POST') {
      await route.fallback();
      return;
    }
    const message = request.postDataJSON() as {
      id: number;
      method: string;
      params: Record<string, unknown>;
    };
    calls.push(message.method);
    const reply = (result: unknown): Promise<void> =>
      route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
      });
    switch (message.method) {
      case 'space/list':
        return reply({ spaces: [SPACE] });
      case 'thread/list':
        return reply({ threads: [thread(THREAD, 'Make the tests pass')] });
      case 'thread/get':
        return reply({ thread: thread(String(message.params['thread']), 'Make the tests pass') });
      case 'thread/events':
        return reply({ events: message.params['thread'] === THREAD ? HISTORY : [] });
      case 'thread/start':
        return reply({ thread: thread(NEW_THREAD, '') });
      case 'tasks/list':
        return reply({ tasks: [] });
      case 'memory/list':
        return reply({ memories: [] });
      case 'server/discover':
        return reply({ name: 'glave', version: '0.0.0', protocols: ['2026-10'] });
      case 'turn/start': {
        const target = String(message.params['thread']);
        const after = target === THREAD ? HISTORY.length : 0;
        const notes = [
          {
            jsonrpc: '2.0',
            method: 'thread/event',
            params: {
              thread: target,
              event: event(
                after + 1,
                'message.added',
                { role: 'user', parts: [{ type: 'text', text: String(message.params['prompt']) }] },
                target,
              ),
            },
          },
          {
            jsonrpc: '2.0',
            method: 'turn/live',
            params: { thread: target, update: { kind: 'text', text: 'Hello from ' } },
          },
          {
            jsonrpc: '2.0',
            method: 'turn/live',
            params: { thread: target, update: { kind: 'text', text: 'Hope.' } },
          },
          {
            jsonrpc: '2.0',
            method: 'thread/event',
            params: {
              thread: target,
              event: event(
                after + 2,
                'message.added',
                { role: 'assistant', parts: [{ type: 'text', text: 'Hello from Hope.' }] },
                target,
              ),
            },
          },
          {
            jsonrpc: '2.0',
            id: message.id,
            result: {
              turn: { outcome: 'completed', model_calls: 1, tool_calls: 0, tokens: TOKENS },
            },
          },
        ];
        return route.fulfill({
          contentType: 'text/event-stream',
          body: notes.map((note) => `data: ${JSON.stringify(note)}\n\n`).join(''),
        });
      }
      default:
        return reply({});
    }
  });
  return { calls };
}
