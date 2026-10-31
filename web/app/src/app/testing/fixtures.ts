import type {
  ChangedFile,
  ModelInfo,
  RawEvent,
  RunningCommand,
  ScheduleInfo,
  Project,
  TaskInfo,
  ThreadChanges,
  ThreadSummary,
  TokenCounts,
} from '@glave/client';

/** Test data shaped as the session API sends it. */

export const TOKENS: TokenCounts = {
  input: 1200,
  output: 300,
  cache_read: 0,
  cache_write: 0,
  reasoning: 0,
};

export function record(id: string, name: string, kind: Project['kind'] = 'project'): Project {
  return {
    id,
    kind,
    name,
    owner: { kind: 'person', id: 'person-1' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:00:00.000Z',
  };
}

export function thread(
  id: string,
  title: string | null,
  overrides: Partial<ThreadSummary> = {},
): ThreadSummary {
  return {
    id,
    project: 'project-1',
    status: 'idle',
    standing: 'active',
    title,
    permissions: 'auto',
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:05:00.000Z',
    turns: 1,
    tokens: TOKENS,
    ...overrides,
  };
}

export function event(
  seq: number,
  type: string,
  data: Readonly<Record<string, unknown>>,
  threadId = 'thread-1',
): RawEvent {
  return {
    glave: 1,
    id: `event-${String(seq)}`,
    thread: threadId,
    seq,
    at: '2026-10-11T10:00:00.000Z',
    type,
    v: 1,
    data,
  };
}

/** What the server answers a thread changed: `files`, all in the turn that started at `turn`. */
export function changed(
  turn: number,
  files: readonly ChangedFile[],
): { readonly changes: ThreadChanges } {
  const note = 'Changes made by commands or other tools are not listed.';
  return { changes: { files, turns: [{ turn, files }], note } };
}

/** A file the server says one edit changed, taking out `removed` and putting in `added`. */
export function editedFile(
  path: string,
  removed: readonly string[],
  added: readonly string[],
): ChangedFile {
  const lines = [
    ...removed.map((text) => ({ kind: 'removed', text }) as const),
    ...added.map((text) => ({ kind: 'added', text }) as const),
  ];
  return {
    path,
    added: added.length,
    removed: removed.length,
    changes: [{ kind: 'edited', lines }],
  };
}

export function task(threadId: string, title: string, overrides: Partial<TaskInfo> = {}): TaskInfo {
  return {
    thread: threadId,
    project: 'project-1',
    kind: 'agent',
    title,
    model: 'hope',
    tokens: TOKENS,
    cost: { micros: 4200, currency: 'USD' },
    started_at: '2026-10-11T10:00:00.000Z',
    elapsed_ms: 125_000,
    status: 'running',
    ...overrides,
  };
}

/** A command running in the background, as `command/list` lists one, started at 10:00. */
export function command(
  id: string,
  script: string,
  overrides: Partial<RunningCommand> = {},
): RunningCommand {
  return {
    id,
    command: script,
    started_at: '2026-10-11T10:00:00.000Z',
    summary: 'Still running: the agent started it, and it runs until it ends or someone stops it.',
    ...overrides,
  };
}

/** A schedule, with its next run or none (a one-off that has run). */
export function schedule(id: string, next: string | null): ScheduleInfo {
  return {
    schedule: id,
    prompt: 'Audit the dependencies',
    when: { cron: '0 2 * * *' },
    budget: 100_000,
    created_at: '2026-10-11T10:00:00.000Z',
    next,
  };
}

/** A server's models: the default, and Hope. */
export const MODELS: readonly ModelInfo[] = [
  { id: 'gpt-6-luna', name: 'GPT-6 Luna', default: true },
  { id: 'hope-12b', name: 'Hope', default: false },
];

/** Models a router serves, each provider as the server says it, some hidden by the filters. */
export const ROUTED: readonly ModelInfo[] = [
  {
    id: 'deepseek/deepseek-v4.1-flash',
    name: 'DeepSeek V4.1 Flash',
    maker: 'DeepSeek',
    default: true,
    context: 131_072,
    endpoints: [
      {
        provider: 'DeepSeek',
        slug: 'deepseek',
        based_in: 'CN',
        precision: 'eight_bit',
        keeps_nothing: false,
        price: { input: 100_000, output: 400_000 },
        speed: 90,
        hidden_by: ['based_in'],
      },
      {
        provider: 'Groq',
        slug: 'groq',
        based_in: 'US',
        datacenters: ['US'],
        precision: 'four_bit',
        keeps_nothing: true,
        price: { input: 120_000, output: 500_000 },
        speed: 210,
      },
      {
        provider: 'Fireworks',
        slug: 'fireworks',
        based_in: 'US',
        precision: 'eight_bit',
        keeps_nothing: true,
        price: { input: 150_000, output: 600_000 },
        speed: 130,
      },
    ],
  },
  { id: 'claude-haiku-5-5', name: 'Claude Haiku 5.5', maker: 'Anthropic', default: false },
];

/** What `server/discover` says. */
export const SERVER = {
  name: 'glave',
  version: '0.1.0',
  protocols: ['2026-10'],
  permissions: 'auto',
  modes: ['auto', 'plan', 'ask', 'full-access'],
} as const;

/** What a device answers when it makes a passkey, shaped as the browser's answers are. */
export const MADE_PASSKEY = {
  id: 'BAU',
  rawId: new Uint8Array([4, 5]).buffer,
  type: 'public-key',
  response: {
    attestationObject: new Uint8Array([1]).buffer,
    clientDataJSON: new Uint8Array([2]).buffer,
    getTransports: () => ['internal'],
  },
  getClientExtensionResults: () => ({}),
};

/** What the server answers when a passkey is to be made at `glave.test`. */
export const PASSKEY_OPTIONS = {
  ceremony: 'c1',
  options: {
    publicKey: {
      challenge: 'AQID',
      rp: { id: 'glave.test', name: 'glave' },
      user: { id: 'AQID', name: 'glave', displayName: 'glave' },
      pubKeyCredParams: [],
    },
  },
};
