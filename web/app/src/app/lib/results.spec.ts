import type { Outcome, TurnResult } from '@glave/client';

import {
  NO_THOUGHT,
  type ApprovalItem,
  type CallItem,
  type MessageItem,
  type QuestionItem,
  type ServiceItem,
  type ThreadItem,
} from './conversation';
import { type TurnFacts, turnResults } from './results';
import { NO_STEPS, type Steps, type TurnPart } from './runs';
import type { Turn } from './turns';

const ask: MessageItem = {
  kind: 'message',
  files: [],
  key: 'm1',
  seq: 1,
  from: 'person',
  text: 'Fix it',
  reasoning: '',
  thought: NO_THOUGHT,
  steered: false,
};

function say(seq: number, text: string, reasoning = ''): MessageItem {
  return {
    kind: 'message',
    files: [],
    key: `m${String(seq)}`,
    seq,
    from: 'model',
    text,
    reasoning,
    thought: NO_THOUGHT,
    steered: false,
  };
}

function call(seq: number, status: CallItem['status'] = 'done'): CallItem {
  return {
    kind: 'call',
    key: `c${String(seq)}`,
    seq,
    call: `c${String(seq)}`,
    tool: 'bash',
    summary: 'cargo test',
    status,
    output: '',
  };
}

function approval(seq: number, decided: boolean): ApprovalItem {
  return {
    kind: 'approval',
    key: `a${String(seq)}`,
    seq,
    approval: `c${String(seq)}`,
    tool: 'bash',
    summary: 'bash: git push',
    ask: 'Run this command?',
    detail: 'git push',
    caution: 'It pushes to a remote.',
    outsideSandbox: false,
    withNetwork: false,
    modelReason: '',
    decision: decided ? 'allow_once' : null,
  };
}

function outside(seq: number, decision: ApprovalItem['decision']): ApprovalItem {
  return {
    ...approval(seq, false),
    summary: 'bash: cargo add serde',
    caution: 'It runs outside the sandbox.',
    outsideSandbox: true,
    decision,
  };
}

function withNetwork(seq: number, decision: ApprovalItem['decision']): ApprovalItem {
  return {
    ...outside(seq, decision),
    summary: 'bash: npm install',
    caution: 'It can reach the internet, which commands here otherwise cannot.',
    withNetwork: true,
  };
}

function service(seq: number, told: string | null = null): ServiceItem {
  return {
    kind: 'service',
    key: `service-${String(seq)}`,
    seq,
    call: 's',
    service: 'GitHub',
    ask: 'GitHub asks you, not the model',
    note: 'Your answer goes only to GitHub.',
    message: 'Which?',
    fields: [],
    told,
  };
}

function question(seq: number): QuestionItem {
  return { kind: 'question', key: `q${String(seq)}`, seq, call: 'q', questions: [], answers: null };
}

function turn(items: readonly ThreadItem[], more: Partial<Turn> = {}): Turn {
  return {
    key: 't',
    items,
    startSeq: null,
    took: null,
    started: null,
    model: null,
    result: null,
    ...more,
  };
}

/** A turn's result as the server records it at its end: free, nothing let out, unless `more`. */
function ended(outcome: Outcome, more: Partial<TurnResult> = {}): TurnResult {
  return {
    outcome,
    model_calls: 1,
    tool_calls: 1,
    outside_sandbox: 0,
    with_network: 0,
    cost: { micros: 0, currency: 'USD' },
    estimated: false,
    via: [],
    ...more,
  };
}

/** What the server says of a turn: `runs` of call ids, each told in `sentence`, and `replies`. */
function told(
  runs: readonly (readonly [string, readonly string[]])[],
  replies: readonly number[] = [],
  more: Partial<TurnFacts> = {},
): TurnFacts {
  const steps: Steps = {
    runs: new Map(
      runs.flatMap(([sentence, calls]) =>
        // Keyed by the `seq` of its first call, as the server keys it: `c3` was asked at 3.
        calls.map((id) => [id, { key: `run-${(calls[0] ?? '').slice(1)}`, sentence }] as const),
      ),
    ),
    replies: new Set(replies),
  };
  return { sourced: new Map(), changed: new Map(), steps, models: new Map(), ...more };
}

/** Each part in a line: a run as its line (`untold` until told) and what it holds, else its key. */
function drawn(parts: readonly TurnPart[]): readonly string[] {
  return parts.map((part) =>
    part.kind === 'run'
      ? `${part.sentence ?? 'untold'}: ${part.items.map((item) => item.key).join(' ')}`
      : part.key,
  );
}

describe('results', () => {
  it('shows a working turn in order: what it said, each run of calls in the server’s line', () => {
    const shown = turnResults(
      turn(
        [
          ask,
          say(2, 'Looking'),
          call(3),
          approval(3, true),
          say(4, '', 'hm'),
          call(5),
          say(6, 'Fixed'),
        ],
        {
          took: 242_000,
          model: 'anthropic/claude-haiku-5-5',
          result: ended('completed', {
            cost: { micros: 42_000, currency: 'USD' },
            via: ['OpenRouter (Anthropic)'],
          }),
        },
      ),
      null,
      null,
      false,
      'en-US',
      told([['Ran 2 commands', ['c3', 'c5']]], [6], {
        models: new Map([['anthropic/claude-haiku-5-5', 'Claude Haiku 5.5']]),
      }),
    );
    expect(shown.prompt).toBe(ask);
    // An approval answered sits beside its call, the reasoning between calls inside their run.
    expect(drawn(shown.parts)).toEqual(['m2', 'Ran 2 commands: c3 a3 m4 c5', 'm6']);
    const interim = shown.parts.map((part) => part.kind === 'item' && part.interim);
    expect(interim).toEqual([true, false, false]);
    expect(shown.receipt).toEqual({
      status: 'done',
      facts: ['4m 02s', '$0.04', 'Claude Haiku 5.5'],
      details: ['via OpenRouter (Anthropic)'],
      steps: '2 steps',
    });
    expect(shown.waiting).toEqual([]);
  });

  it('draws each run as its line from the start, its words to come until the server tells it', () => {
    const shown = turnResults(
      turn([ask, say(2, '', 'hm'), call(3), call(4)]),
      null,
      null,
      true,
      'en-US',
      told([['Ran 1 command', ['c3']]]),
    );
    // A call the server has not told yet joins its run, under the line already told.
    expect(drawn(shown.parts)).toEqual(['m2', 'Ran 1 command: c3 c4']);
    // Untold, words, a question or a service's end a run, as the server reads them; reasoning does not.
    const untold = turnResults(
      turn([
        ask,
        call(2),
        say(3, '', 'hm'),
        call(4),
        say(5, 'Looking'),
        call(6),
        { ...question(7), answers: ['Yes'] },
        call(8),
        service(9, 'You answered GitHub'),
        call(10),
      ]),
      null,
      null,
      true,
      'en-US',
    );
    expect(drawn(untold.parts)).toEqual([
      'untold: c2 m3 c4',
      'm5',
      'untold: c6',
      'q7',
      'untold: c8',
      'service-9',
      'untold: c10',
    ]);
    // Keyed as the server will tell them, so nothing is drawn again when it does.
    const keys = untold.parts.filter((part) => part.kind === 'run').map((part) => part.key);
    expect(keys).toEqual(['run-2', 'run-6', 'run-8', 'run-10']);
    expect(NO_STEPS.runs.size).toBe(0);
  });

  it('gives the reply what it drew on, by its seq', () => {
    const sourced = {
      reply: 4,
      sentence: 'Sources: 1 page',
      pages: [{ url: 'https://tokio.rs/blog', title: 'Tokio blog' }],
    };
    const facts = told([], [4], { sourced: new Map([[4, sourced]]) });
    const items = [ask, say(2, 'Looking'), call(3), say(4, 'Here')];
    const sources = (shown: readonly TurnPart[]): unknown =>
      shown.map((part) => (part.kind === 'item' ? part.sources : null));
    const shown = turnResults(turn(items), null, null, false, 'en-US', facts);
    expect(sources(shown.parts)).toEqual([null, null, sourced]);
  });

  it('gives a turn the files the server says it changed, by the seq of its start', () => {
    const changed = [{ path: 'src/a.rs', added: 2, removed: 1, changes: [] }];
    const facts = told([], [], { changed: new Map([[7, changed]]) });
    const items = [ask, call(8), say(9, 'Done')];
    expect(turnResults(turn(items, { startSeq: 7 }), null, null, false, 'en-US', facts).files).toBe(
      changed,
    );
    expect(
      turnResults(turn(items, { startSeq: 3 }), null, null, false, 'en-US', facts).files,
    ).toEqual([]);
    expect(turnResults(turn(items), null, null, false, 'en-US', facts).files).toEqual([]);
    // Until the server has said, as the thread opens, none are known.
    const unknown = told([], [], { changed: null });
    const opening = turnResults(turn(items, { startSeq: 7 }), null, null, false, 'en-US', unknown);
    expect(opening.files).toBeNull();
  });

  it('says how many commands ran outside the sandbox as the server counted them', () => {
    const items = [ask, call(2), outside(3, 'allow_once'), call(4), outside(5, 'deny')];
    const shown = turnResults(
      turn([...items, say(6, 'Added')], {
        took: 2_000,
        result: ended('completed', { outside_sandbox: 1 }),
      }),
      null,
      null,
      false,
      'en-US',
    );
    expect(shown.receipt?.facts).toEqual(['2s', '$0.00', '1 command outside the sandbox']);
    // Never counted from the approvals: the turn still running has no result yet.
    const running = turnResults(turn(items), null, null, false, 'en-US');
    expect(running.receipt?.facts).toEqual([]);
  });

  it('says the commands that ran with the network apart from those outside the sandbox', () => {
    const items = [ask, withNetwork(2, 'allow_once'), withNetwork(3, 'allow_once'), call(4)];
    const result = ended('completed', { outside_sandbox: 1, with_network: 2 });
    const shown = turnResults(turn(items, { result }), null, null, false, 'en-US');
    expect(shown.receipt?.facts).toEqual([
      '$0.00',
      '1 command outside the sandbox',
      '2 commands with the network',
    ]);
  });

  it('says a cost the server did not have is not reported, and one it estimated so', () => {
    const facts = (result: TurnResult): unknown =>
      turnResults(turn([ask, call(2)], { result }), null, null, false, 'en-US').receipt?.facts;
    expect(facts(ended('completed', { cost: null }))).toEqual(['cost not reported']);
    const guessed = ended('completed', { cost: { micros: 1_230_000, currency: 'USD' } });
    expect(facts({ ...guessed, estimated: true })).toEqual(['$1.23 (estimated)']);
    const via = ended('completed', { via: ['Ollama', 'OpenRouter (Anthropic)'] });
    expect(facts(via)).toEqual(['$0.00']);
    const routes = turnResults(turn([ask, call(2)], { result: via }), null, null, false, 'en-US');
    expect(routes.receipt?.details).toEqual(['via Ollama, OpenRouter (Anthropic)']);
  });

  it('shows what waits for the person last, never folded', () => {
    const waiting = approval(4, false);
    const shown = turnResults(
      turn([ask, call(2), say(3, 'Ready to push'), waiting]),
      null,
      null,
      true,
      'en-US',
    );
    expect(drawn(shown.parts)).toEqual(['untold: c2', 'm3']);
    expect(shown.waiting).toEqual([waiting]);
    expect(shown.receipt?.status).toBe('waiting');
    expect(shown.receipt?.facts).toEqual([]);
  });

  it('shows a turn that did no work as what it said, with no receipt', () => {
    const shown = turnResults(
      turn([ask, say(2, '', 'thinking'), say(3, 'Hello')]),
      null,
      null,
      false,
      'en-US',
    );
    expect(drawn(shown.parts)).toEqual(['m2', 'm3']);
    expect(shown.receipt).toBeNull();
  });

  it('says how a turn went: failed, stopped, still going, or done', () => {
    const status = (
      more: Partial<Turn>,
      running = false,
      items: readonly ThreadItem[] = [ask, call(2)],
    ): unknown => turnResults(turn(items, more), null, null, running, 'en-US').receipt?.status;
    expect(status({ result: ended('failed') })).toBe('failed');
    expect(status({ result: ended('cancelled') })).toBe('stopped');
    expect(status({ result: ended('limit_reached') })).toBe('stopped');
    expect(status({ result: ended('completed') }, true)).toBe('done');
    expect(status({}, true)).toBe('running');
    expect(status({}, false, [ask, call(2, 'running')])).toBe('running');
    expect(status({}, false, [ask, question(2)])).toBe('waiting');
    expect(status({}, false, [ask, call(2, 'running'), service(3)])).toBe('waiting');
    expect(status({ result: ended('completed') }, false, [ask, call(2), service(3, 'told')])).toBe(
      'done',
    );
    expect(status({})).toBe('done');
    expect(turnResults(turn([ask, call(2)]), null, null, false, 'en-US').receipt?.steps).toBe(
      '1 step',
    );
  });

  it('writes what the model writes now after everything recorded, which stays in place', () => {
    const live = { text: 'Writ', reasoning: '', thought: NO_THOUGHT };
    const shown = turnResults(turn([ask, call(2), say(3, 'Earlier')]), live, null, true, 'en-US');
    expect(shown.live).toBe(live);
    expect(drawn(shown.parts)).toEqual(['untold: c2', 'm3']);
  });

  it('is working while the turn runs with nothing of its own to show', () => {
    const working = (items: readonly ThreadItem[], more: Partial<Turn> = {}): boolean =>
      turnResults(turn(items, more), null, null, true, 'en-US').working !== null;
    // Just asked, while a call runs, and between a call's end and the model's next words.
    expect(working([ask])).toBe(true);
    expect(working([ask, call(2, 'running')])).toBe(true);
    expect(working([ask, say(2, ''), call(3)])).toBe(true);
    // Not while something waits for the person, or after the model's last words.
    expect(working([ask, approval(2, false)])).toBe(false);
    expect(working([ask, call(2), say(3, 'Done.')])).toBe(false);
    // Not once it ended, nor in a turn that is not running.
    expect(working([ask], { result: ended('failed') })).toBe(false);
    expect(turnResults(turn([ask]), null, null, false, 'en-US').working).toBeNull();
    // Not while reasoning or a reply streams: they show instead.
    expect(
      turnResults(
        turn([ask]),
        { text: '', reasoning: 'Hm', thought: NO_THOUGHT },
        null,
        true,
        'en-US',
      ).working,
    ).toBeNull();
  });

  it('shows the step a running call takes in the server’s words, and since when it has worked', () => {
    const started = '2026-10-11T10:00:00.000Z';
    const step = 'bash: cargo test';
    const shown = (items: readonly ThreadItem[]): unknown =>
      turnResults(turn(items, { started }), null, step, true, 'en-US').working;
    expect(shown([ask, call(2, 'running')])).toEqual({ step, since: started });
    // Once no call runs, its model thinks: whatever the server last said, there is no step.
    expect(shown([ask, call(2)])).toEqual({ step: null, since: started });
    // Until the server records the turn's start, there is no time to count from.
    expect(turnResults(turn([ask]), null, null, true, 'en-US').working).toEqual({
      step: null,
      since: null,
    });
  });

  it('says how long it took, but for while the working line counts it', () => {
    const facts = (
      running: boolean,
      more: Partial<Turn> = {},
      items: readonly ThreadItem[] = [ask, call(2)],
    ): unknown =>
      turnResults(turn(items, { took: 3_000, ...more }), null, null, running, 'en-US').receipt
        ?.facts;
    expect(facts(true)).toEqual([]);
    // Waiting for the person, no line counts: the time so far.
    expect(facts(true, {}, [ask, call(2), approval(3, false)])).toEqual(['3s']);
    expect(facts(true, { result: ended('completed') })).toEqual(['3s', '$0.00']);
    expect(facts(false)).toEqual(['3s']);
  });
});
