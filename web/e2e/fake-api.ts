import type { Page, Route } from '@playwright/test';

/**
 * What the fake server knows: a project and the person's own space with their threads, one
 * thread's history (a command, an edit and a new file, a reply from the default model, which
 * the picker shows), the models, the tasks, and what a turn streams.
 */
const OWNER = { kind: 'person', id: '01900000-0000-7000-8000-0000000000aa' };

function space(id: string, name: string, kind: string): Record<string, unknown> {
  return {
    id,
    kind,
    name,
    owner: OWNER,
    members: [],
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:00:00.000Z',
  };
}

const PROJECT = space('01900000-0000-7000-8000-000000000001', 'glave', 'project');
const PERSONAL = space('01900000-0000-7000-8000-000000000002', 'Personal', 'personal');

const TOKENS = { input: 1200, output: 300, cache_read: 0, cache_write: 0, reasoning: 0 };

function thread(
  id: string,
  title: string,
  status = 'idle',
  inSpace = PROJECT,
): Record<string, unknown> {
  return {
    id,
    space: inSpace['id'],
    status,
    standing: 'active',
    title,
    approval: 'auto',
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:05:00.000Z',
    turns: 1,
    tokens: TOKENS,
  };
}

export const THREAD = '01900000-0000-7000-8000-000000000101';
export const NEW_THREAD = '01900000-0000-7000-8000-000000000102';

const THREADS = {
  [PROJECT['id'] as string]: [
    thread(THREAD, 'Make the tests pass'),
    thread('01900000-0000-7000-8000-000000000103', 'Review the release notes', 'waiting'),
    thread('01900000-0000-7000-8000-000000000104', 'Nightly dependency audit', 'running'),
  ],
  [PERSONAL['id'] as string]: [
    thread('01900000-0000-7000-8000-000000000105', 'Dinner ideas for Saturday', 'idle', PERSONAL),
  ],
};

/** The default model: no name configured, so the app names it from its id. */
export const DEFAULT_MODEL = 'anthropic/claude-haiku-5.5';

/**
 * The models the fake server runs: its default, Hope, two more, and a long catalogue like
 * OpenRouter's, which the picker must search and scroll.
 */
export const MODELS = [
  { id: DEFAULT_MODEL, default: true, hope: false },
  { id: 'hope-12b', name: 'Hope', default: false, hope: true },
  { id: 'openai/gpt-6-luna', default: false, hope: false },
  { id: 'deepseek/deepseek-v4.1-flash', default: false, hope: false },
  ...Array.from({ length: 200 }, (_, index) => ({
    id: `lab-${String(index % 20)}/model-${String(index)}`,
    default: false,
    hope: false,
  })),
];

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

function call(seq: number, callId: string, tool: string, json: Record<string, unknown>): unknown {
  return event(seq, 'tool.call.requested', { call_id: callId, tool, arguments: { json } });
}

function done(seq: number, callId: string, text: string): unknown {
  return event(seq, 'tool.call.completed', {
    call_id: callId,
    is_error: false,
    output: [{ type: 'text', text }],
  });
}

const HISTORY = [
  event(1, 'thread.started', {}),
  event(2, 'message.added', {
    role: 'user',
    parts: [{ type: 'text', text: 'Make the tests pass' }],
  }),
  event(3, 'turn.started', { model: { provider: 'e2e', name: DEFAULT_MODEL } }),
  call(4, 'call_1', 'bash', { command: 'cargo test' }),
  done(5, 'call_1', 'test parse::empty ... FAILED'),
  call(6, 'call_2', 'edit', {
    path: 'src/parse.rs',
    old_string: 'fn parse(input: &str) -> Value {\n    input.parse().unwrap()\n}',
    new_string:
      'fn parse(input: &str) -> Option<Value> {\n    if input.is_empty() {\n        return None;\n    }\n    input.parse().ok()\n}',
  }),
  done(7, 'call_2', 'Edited src/parse.rs'),
  call(8, 'call_3', 'write', {
    path: 'tests/empty.rs',
    content: '#[test]\nfn empty_input_is_none() {\n    assert_eq!(parse(""), None);\n}\n',
  }),
  done(9, 'call_3', 'Wrote tests/empty.rs'),
  call(10, 'call_4', 'bash', { command: 'cargo test' }),
  done(11, 'call_4', 'test result: ok. 12 passed'),
  event(12, 'message.added', {
    role: 'assistant',
    parts: [
      {
        type: 'text',
        text: 'They pass now. parse() returned a value for empty input and unwrapped a failed parse; it now returns None for both, and a new test covers the empty case.',
      },
    ],
  }),
  event(13, 'model.responded', {
    model: { provider: 'e2e', name: DEFAULT_MODEL },
    stop: 'end_turn',
  }),
];

const TASKS = [
  {
    thread: '01900000-0000-7000-8000-000000000104',
    kind: 'agent',
    title: 'Nightly dependency audit',
    model: 'hope-12b',
    tokens: { ...TOKENS, input: 48200, output: 3100 },
    cost: { micros: 21500, currency: 'USD' },
    started_at: '2026-10-11T10:00:00.000Z',
    elapsed_ms: 754_000,
    status: 'running',
  },
  {
    thread: '01900000-0000-7000-8000-000000000106',
    kind: 'agent',
    title: 'Summarize open issues',
    model: DEFAULT_MODEL,
    tokens: TOKENS,
    cost: { micros: 4200, currency: 'USD' },
    started_at: '2026-10-11T09:00:00.000Z',
    elapsed_ms: 125_000,
    status: 'finished',
    outcome: 'completed',
  },
];

/** A request the fake answered: its method and params. */
export interface Request {
  readonly method: string;
  readonly params: Record<string, unknown>;
}

/** What a turn streams: the person's message, the reply growing, then the reply itself. */
function turn(message: { id: number; params: Record<string, unknown> }): string {
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
      params: { thread: target, update: { kind: 'text', text: 'Glad to ' } },
    },
    {
      jsonrpc: '2.0',
      method: 'turn/live',
      params: { thread: target, update: { kind: 'text', text: 'help.' } },
    },
    {
      jsonrpc: '2.0',
      method: 'thread/event',
      params: {
        thread: target,
        event: event(
          after + 2,
          'message.added',
          { role: 'assistant', parts: [{ type: 'text', text: 'Glad to help.' }] },
          target,
        ),
      },
    },
    {
      jsonrpc: '2.0',
      id: message.id,
      result: { turn: { outcome: 'completed', model_calls: 1, tool_calls: 0, tokens: TOKENS } },
    },
  ];
  return notes.map((note) => `data: ${JSON.stringify(note)}\n\n`).join('');
}

/** The answer to a call that is not a turn. */
function answer(method: string, params: Record<string, unknown>): unknown {
  switch (method) {
    case 'space/list':
      return { spaces: [PROJECT, PERSONAL] };
    case 'thread/list':
      return { threads: THREADS[String(params['space'])] ?? [] };
    case 'thread/get':
      return { thread: thread(String(params['thread']), 'Make the tests pass') };
    case 'thread/events':
      return { events: params['thread'] === THREAD ? HISTORY : [] };
    case 'thread/start':
      return { thread: thread(NEW_THREAD, '') };
    case 'thread/rename':
      return { thread: thread(String(params['thread']), String(params['title'])) };
    case 'model/list':
      return { models: MODELS };
    case 'tasks/list':
      return { tasks: TASKS };
    case 'memory/list':
      return { memories: [] };
    case 'server/discover':
      return {
        name: 'glave',
        version: '0.0.0',
        protocols: ['2026-10'],
        approval: 'auto',
        approvals: ['ask', 'auto'],
      };
    default:
      return {};
  }
}

/** Answers the session API's calls the app makes, as JSON-RPC over POST /. */
export async function fakeApi(
  page: Page,
): Promise<{ readonly calls: readonly string[]; readonly requests: readonly Request[] }> {
  const calls: string[] = [];
  const requests: Request[] = [];
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
    requests.push({ method: message.method, params: message.params });
    if (message.method === 'turn/start') {
      await route.fulfill({ contentType: 'text/event-stream', body: turn(message) });
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: message.id,
        result: answer(message.method, message.params),
      }),
    });
  });
  return { calls, requests };
}
