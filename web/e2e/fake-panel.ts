import type { PlanInfo, ReplySources, RunningCommand, ThreadSummary } from '@glave/client';

import { bound, call, done, event } from './fake-records';

/**
 * A coding thread with something in every section of its panel (`fakeApi`'s `panel`): it read
 * a page, was given a screenshot, changed two files, builds a plan and keeps the tests running.
 */
export const VERBOSE = '01900000-0000-7000-8000-000000000131';

/** The page it read, as `web_fetch` recorded it. */
const PAGE = {
  url: 'https://docs.rs/clap/latest/clap/struct.Arg.html',
  title: 'clap: Arg::action',
};

/** What it keeps running in the background. */
export const TESTS: RunningCommand = {
  id: '01900000-0000-7000-8000-0000000000d2',
  command: 'cargo test',
  started_at: '2026-10-11T10:04:00.000Z',
  summary: 'Still running: the agent started it, and it runs until it ends or someone stops it.',
};

/** What its summary says beyond any thread's: how full its context is, and what it cost. */
export const VERBOSE_SUMMARY: Partial<ThreadSummary> = {
  turns: 2,
  cost: { micros: 12_000, currency: 'USD' },
  context: { percent: 39, tokens: 49_920, window: 128_000 },
};

/** Its plan, being built: a step done, one doing, two to do. */
const PLAN: PlanInfo = {
  proposed: false,
  sentence: 'Plan: 1 of 4 steps done',
  steps: [
    { step: 'Read how src/args.rs parses flags', status: 'completed' },
    { step: 'Add --verbose and -v to Args, printing each step it takes', status: 'in_progress' },
    { step: 'Run cargo test', status: 'pending' },
    { step: 'Say what --verbose does in the README', status: 'pending' },
  ],
};

const ARGS_BEFORE = 'pub struct Args {\n    pub path: PathBuf,\n}';
const ARGS_AFTER = [
  'pub struct Args {',
  '    pub path: PathBuf,',
  '    /// Print each step as it happens.',
  '    #[arg(short, long)]',
  '    pub verbose: bool,',
  '}',
].join('\n');

/** Its history: a look at clap's docs, then the change, the tests started in the background. */
export const VERBOSE_HISTORY = [
  event(1, 'thread.started', {}, VERBOSE),
  bound(2, VERBOSE),
  event(3, 'turn.started', {}, VERBOSE),
  event(
    4,
    'message.added',
    {
      role: 'user',
      parts: [
        { type: 'text', text: 'Add a --verbose flag to the CLI' },
        { type: 'file', name: 'terminal-output.png', blob: { bytes: 48_213 } },
      ],
    },
    VERBOSE,
  ),
  call(5, 'call_1', 'web_fetch', { url: PAGE.url }, `web_fetch ${PAGE.url}`, VERBOSE),
  {
    ...done(6, 'call_1', `URL: ${PAGE.url}\n\nThe page.`, VERBOSE),
    data: { call_id: 'call_1', is_error: false, output: [], structured: { page: PAGE } },
  },
  call(
    7,
    'call_2',
    'edit',
    { path: 'src/args.rs', old_string: ARGS_BEFORE, new_string: ARGS_AFTER },
    'edit src/args.rs',
    VERBOSE,
  ),
  done(8, 'call_2', 'Edited src/args.rs', VERBOSE),
  call(
    9,
    'call_3',
    'edit',
    {
      path: 'src/main.rs',
      old_string: '    run(args)',
      new_string:
        '    if args.verbose {\n        eprintln!("checking {}", args.path.display());\n    }\n    run(args)',
    },
    'edit src/main.rs',
    VERBOSE,
  ),
  done(10, 'call_3', 'Edited src/main.rs', VERBOSE),
  call(
    11,
    'call_4',
    'bash',
    { command: TESTS.command, background: true },
    `bash: ${TESTS.command}`,
    VERBOSE,
  ),
  done(12, 'call_4', 'Running in the background as command 1.', VERBOSE),
  event(13, 'plan.updated', { steps: PLAN.steps, call_id: 'call_5' }, VERBOSE),
  event(
    14,
    'command.started',
    { command: TESTS.id, call_id: 'call_4', script: TESTS.command },
    VERBOSE,
  ),
  event(
    15,
    'message.added',
    {
      role: 'assistant',
      parts: [
        { type: 'text', text: '`--verbose` and `-v` print each step; the tests are running.' },
      ],
    },
    VERBOSE,
  ),
  event(16, 'turn.ended', { outcome: 'completed' }, VERBOSE, '2026-10-11T10:04:05.000Z'),
];

const SOURCES: readonly ReplySources[] = [
  { reply: 15, sentence: 'Sources: 1 page', pages: [PAGE] },
];

/** The answer to a call about the coding thread, else `undefined`. */
export function aboutVerbose(method: string, params: Record<string, unknown>): unknown {
  if (params['thread'] !== VERBOSE) {
    return undefined;
  }
  switch (method) {
    case 'thread/sources':
      return { sources: SOURCES };
    case 'command/list':
      return { commands: [TESTS] };
    case 'thread/plan':
      return { plan: PLAN };
    default:
      return undefined;
  }
}

/** A plain chat: words alone, so no panel. */
export const DINNER = '01900000-0000-7000-8000-000000000105';

/** The chat's history: a question and a reply. */
export const DINNER_HISTORY = [
  event(1, 'thread.started', {}, DINNER),
  event(2, 'turn.started', {}, DINNER),
  event(
    3,
    'message.added',
    {
      role: 'user',
      parts: [{ type: 'text', text: 'Dinner ideas for Saturday? Six people, one vegetarian.' }],
    },
    DINNER,
  ),
  event(
    4,
    'message.added',
    {
      role: 'assistant',
      parts: [
        {
          type: 'text',
          text: [
            'Three that feed six and work for everyone:',
            '',
            '1. **Taco bar:** spiced beef and black beans side by side, every topping in bowls.',
            '2. **Mushroom and spinach lasagne:** one big dish, a green salad and garlic bread.',
            '3. **Curry night:** a chickpea curry and a chicken curry, rice and naan.',
            '',
            'The taco bar is the least work on the day.',
          ].join('\n'),
        },
      ],
    },
    DINNER,
  ),
  event(5, 'turn.ended', { outcome: 'completed' }, DINNER, '2026-10-11T10:00:09.000Z'),
];
