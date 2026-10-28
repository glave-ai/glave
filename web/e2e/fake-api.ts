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

import { type Commands, SERVING, SERVING_HISTORY, background } from './fake-commands';
import { type Awake, keptAwake } from './fake-awake';
import { changesOf } from './fake-changes';
import { stepsOf } from './fake-steps';
import { keptFile } from './fake-files';
import { HOME, listFolders } from './fake-folders';
import { type Gated, gates } from './fake-gate';
import { type Grouped, keptGrouping } from './fake-grouping';
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
import {
  DINNER,
  DINNER_HISTORY,
  VERBOSE,
  VERBOSE_HISTORY,
  VERBOSE_SUMMARY,
  aboutVerbose,
} from './fake-panel';
import { Refusal, SCRATCH, THREAD, WORKSPACE, bound, call, done, event } from './fake-records';
import { discovered, signIn } from './fake-signin';
import { PLANNING, PLANNING_HISTORY, type Planned, planned } from './fake-plan';
import { RESEARCH, RESEARCH_HISTORY, sourcesOf } from './fake-sources';

/**
 * What the fake server knows: a project and the person's own project with their threads, one
 * thread's history (a turn on this computer that ran a command, edited a file and wrote a new
 * one, and replied from the default model, which the picker shows; how long it took and what
 * it cost), another waiting for the person to allow a push, a chat that read pages on the web
 * and what its reply drew on, the models, the tasks, one memory, what a turn streams, the
 * folders on the person's computer, and the projects made while it runs; and when a test asks,
 * a project that asks to be trusted, a thread that keeps the docs site running in the
 * background, a chat among the person's own that read pages, and a connected GitHub account.
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
    permissions: 'auto',
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:05:00.000Z',
    turns: 1,
    tokens: TOKENS,
    // A chat works in a scratch folder of its own.
    workspace: inProject.kind === 'personal' ? SCRATCH : WORKSPACE,
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

/** The chat that read pages on the web, as the person's chats and `thread/get` show it. */
const RESEARCHING = thread(RESEARCH, 'What changed in Tokio 2.0?', 'idle', PERSONAL, {
  updated_at: '2026-10-11T10:07:00.000Z',
});

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
  [PERSONAL.id]: [thread(DINNER, 'Dinner ideas for Saturday', 'idle', PERSONAL)],
};

/** A chat that keeps the docs site running in the background, when a test asks. */
const SERVING_CHAT = thread(SERVING, 'Preview the docs', 'idle', PERSONAL);

/** The coding thread with something in every section of its panel, when a test asks. */
const VERBOSE_THREAD = thread(VERBOSE, 'Add a --verbose flag to the CLI', 'idle', PROJECT, {
  ...VERBOSE_SUMMARY,
  updated_at: '2026-10-11T10:08:00.000Z',
});

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
  event(
    15,
    'turn.ended',
    {
      outcome: 'completed',
      model_calls: 5,
      tool_calls: 4,
      outside_sandbox: 0,
      with_network: 0,
      tokens: { input: 9000, cache_read: 0, cache_write: 0, output: 600, reasoning: 0 },
      cost: { micros: 4200, currency: 'USD' },
      estimated: false,
    },
    THREAD,
    '2026-10-11T10:02:05.000Z',
  ),
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
      ask: 'Run this command?',
      detail: 'git push -u origin docs/release-notes',
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
    { text: 'Lets glave work outside the sandbox without asking' },
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
  [RESEARCH]: RESEARCH_HISTORY,
  [LOST]: LOST_HISTORY,
  [VERBOSE]: VERBOSE_HISTORY,
  [DINNER]: DINNER_HISTORY,
  [PLANNING]: PLANNING_HISTORY,
};

/** The thread that proposed a plan, as the lists show it: in Plan until it is built. */
function planning(plan: Planned): ThreadSummary {
  return thread(PLANNING, 'Add a --verbose flag to the CLI', 'idle', PROJECT, {
    permissions: plan.built ? 'auto' : 'plan',
    updated_at: '2026-10-11T10:08:00.000Z',
  });
}

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
    source: { thread: THREAD, seq: 3 },
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
 * What a turn streams: the level it chose (`effort`, `null` for Default, none when it chose
 * none), the person's message, the reply growing piece by piece, then the reply itself and the
 * turn's result (answering call `id`).
 */
function turnNotes(
  target: string,
  prompt: string,
  pieces: readonly string[],
  id: number,
  effort?: unknown,
): readonly Note[] {
  const chose = effort === undefined ? [] : [effort === null ? {} : { effort }];
  const after = (HISTORIES[target]?.length ?? 0) + chose.length;
  const recorded = (seq: number, type: string, data: Readonly<Record<string, unknown>>): Note => ({
    jsonrpc: '2.0',
    method: 'thread/event',
    params: { thread: target, event: event(seq, type, data, target) },
  });
  const added = (seq: number, role: string, text: string): Note =>
    recorded(seq, 'message.added', { role, parts: [{ type: 'text', text }] });
  return [
    ...chose.map((data) => recorded(after, 'effort.chosen', data)),
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
    message.params['effort'],
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

/** `text` in the pieces a model streams it in, a few characters each. */
function piecesOf(text: string): readonly string[] {
  return text.match(/[\s\S]{1,6}/gu) ?? [];
}

/** Where `words` end in `reply`; fails when it never says them. */
function endOf(reply: string, words: string): number {
  const at = reply.indexOf(words);
  if (at < 0) {
    throw new Error(`the reply never says ${words}`);
  }
  return at + words.length;
}

/**
 * Makes the page's next turn stream `reply` to `THREAD` a few characters at a time, as a model
 * would: the page's own `fetch` answers `turn/start` with a stream that sends each piece on its
 * own, `delay` milliseconds apart, so the page renders the reply as it grows. Given `heldAfter`,
 * words of the reply, the stream waits once it has sent them, at the gate `reply`, until the
 * test lets it go (`letGo`): so the test sees the reply partway for as long as it looks.
 */
export async function streamReply(
  page: Page,
  reply = MARKDOWN_REPLY,
  { delay = 15, heldAfter }: { readonly delay?: number; readonly heldAfter?: string } = {},
): Promise<void> {
  const end = heldAfter === undefined ? reply.length : endOf(reply, heldAfter);
  const first = piecesOf(reply.slice(0, end));
  const notes = turnNotes(THREAD, MARKDOWN_PROMPT, [...first, ...piecesOf(reply.slice(end))], 0);
  // It waits after the person's message and the reply up to those words.
  const held = heldAfter === undefined ? null : 1 + first.length;
  await gates(page);
  await page.addInitScript(
    ({ notes, delay, held }) => {
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
            for (const [index, note] of notes.entries()) {
              if (index === held) {
                await (window as unknown as Gated).glaveGates.passed('reply');
              }
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
    { notes, delay, held },
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
  /** Whether the person's chats list the research chat. */
  readonly research: boolean;
  /** Whether the project lists the coding thread with something in every part of its panel. */
  readonly panel: boolean;
  /** The thread that proposed a plan, and its plan, when the test has it. */
  readonly plan: Planned | null;
  /** The git hosts the person connected. */
  readonly credentials: readonly CredentialInfo[];
  /** The threads given a message, which are then the latest. */
  readonly touched: Set<string>;
  /** Playground, whose folder is gone, when the test has it. */
  readonly gone: Gone | null;
  readonly awake: Awake;
  readonly grouped: Grouped;
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
  const verbose = known.panel ? aboutVerbose(method, params) : undefined;
  if (verbose !== undefined) {
    return verbose;
  }
  const ran =
    background(method, params, commands) ??
    (gone === null ? undefined : aboutGone(method, params, gone)) ??
    (known.plan === null ? undefined : planned(method, params, known.plan));
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
      const serving = commands !== null && project === PERSONAL.id ? [SERVING_CHAT] : [];
      const research = known.research && project === PERSONAL.id ? [RESEARCHING] : [];
      const verbose = known.panel && project === PROJECT.id ? [VERBOSE_THREAD] : [];
      const plans = known.plan !== null && project === PROJECT.id ? [planning(known.plan)] : [];
      const threads = [
        ...[...fresh, ...serving, ...research, ...verbose, ...plans],
        ...(THREADS[project] ?? []),
      ];
      return {
        threads: threads.map((each) =>
          touched.has(each.id) ? { ...each, updated_at: LATER } : each,
        ),
      };
    }
    case 'thread/get': {
      const id = String(params['thread']);
      if (id === PLANNING && known.plan !== null) {
        return { thread: planning(known.plan) };
      }
      const listed = [RESEARCHING, VERBOSE_THREAD, SERVING_CHAT, ...Object.values(THREADS).flat()];
      return { thread: listed.find((each) => each.id === id) ?? thread(id, 'Make the tests pass') };
    }
    case 'thread/sources':
      return sourcesOf(params);
    case 'thread/events':
      return { events: HISTORIES[String(params['thread'])] ?? [] };
    case 'thread/changes':
      return { changes: changesOf(HISTORIES[String(params['thread'])] ?? []) };
    case 'thread/steps':
      return { steps: stepsOf(HISTORIES[String(params['thread'])] ?? []) };
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
      return discovered();
    default:
      return (
        signIn(method, params) ??
        keptAwake(method, params, known.awake) ??
        keptGrouping(method, params, known.grouped) ??
        {}
      );
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
 * person also has acme, a project that asks to be trusted; with `commands`, a chat that keeps
 * the docs site running in the background; with `research`, a chat that read
 * pages on the web among the person's chats; with `plan`, a thread in the project that proposed a
 * plan, whose plan the page answers (`planServer`); with `panel`, a coding thread in the project
 * with something in every section of its panel; with `account`, a GitHub account connected; with
 * `gone`, playground, a project whose folder isn't there anymore. How the person's threads are
 * grouped is kept in `grouped`, which two pages share as one server.
 */
export async function fakeApi(
  page: Page,
  {
    trust = false,
    commands = false,
    research = false,
    plan = false,
    panel = false,
    account = false,
    gone = false,
    grouped = { grouping: 'project' },
  }: {
    readonly trust?: boolean;
    readonly commands?: boolean;
    readonly research?: boolean;
    readonly plan?: boolean;
    readonly panel?: boolean;
    readonly account?: boolean;
    readonly gone?: boolean;
    readonly grouped?: Grouped;
  } = {},
): Promise<{ readonly calls: readonly string[]; readonly requests: readonly Request[] }> {
  const calls: string[] = [];
  const requests: Request[] = [];
  const known: Known = {
    started: [],
    made: [],
    trust: trust ? { asking: true } : null,
    commands: commands ? { running: true } : null,
    research,
    plan: plan ? { built: false, steps: [] } : null,
    panel,
    credentials: account ? [GITHUB] : [],
    touched: new Set(),
    gone: gone ? folderGone() : null,
    awake: { on: true },
    grouped,
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
    const answered = refused ?? answer(message.method, message.params, known);
    const reply =
      answered instanceof Refusal
        ? { error: { code: answered.code, message: answered.message } }
        : { result: answered };
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, ...reply }),
    });
    if (message.method === 'trust/answer' || message.method === 'plan/build') {
      await lists.tell(String(message.params['thread']));
    }
  });
  return { calls, requests };
}
