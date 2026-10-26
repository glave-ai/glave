import type { RawEvent } from '@glave/client';

import {
  NO_DRAFT,
  answersOf,
  applyUpdate,
  argumentsSummary,
  mergeEvents,
  partsText,
  promptOf,
  questionsOf,
  stepsAfter,
  threadItems,
  trimBlankLines,
} from './conversation';

let next = 1;

function event(type: string, data: Record<string, unknown>, seq = next++): RawEvent {
  return {
    glave: 1,
    id: `id-${String(seq)}`,
    thread: 'thread',
    seq,
    at: '2026-10-11T10:00:00.000Z',
    type,
    v: 1,
    data,
  };
}

describe('conversation', () => {
  beforeEach(() => {
    next = 1;
  });

  it('reads text and reasoning out of parts', () => {
    expect(
      partsText([
        { type: 'reasoning', text: 'thinking' },
        { type: 'text', text: 'Hello' },
        { type: 'text', text: 'there' },
        { type: 'file', sha256: 'x' },
      ]),
    ).toEqual({ text: 'Hello\nthere', reasoning: 'thinking' });
    expect(partsText(undefined)).toEqual({ text: '', reasoning: '' });
  });

  it('leaves out the blank lines at the ends of text, which would draw as empty space', () => {
    expect(trimBlankLines('\n \nThe lock\n\nis taken twice.\n\n')).toBe(
      'The lock\n\nis taken twice.',
    );
    expect(trimBlankLines('\r\n    at parse (src/parse.rs)\n')).toBe('    at parse (src/parse.rs)');
    expect(trimBlankLines(' \n\n')).toBe('');
    expect(
      partsText([
        { type: 'reasoning', text: 'Weighing it.\n\n' },
        { type: 'reasoning', text: '\n' },
        { type: 'text', text: 'One.\n\n' },
        { type: 'text', text: 'Two.\n\n' },
      ]),
    ).toEqual({ text: 'One.\n\n\nTwo.', reasoning: 'Weighing it.' });
  });

  it("shows the person's own words, not what glave opened the thread with", () => {
    const parts = [
      { type: 'text', text: '# AGENTS.md instructions for /repo' },
      { type: 'text', text: 'Say hello' },
    ];
    expect(promptOf(parts)).toBe('Say hello');
    expect(promptOf([])).toBe('');
    const [message] = threadItems([event('message.added', { role: 'user', parts })]);
    expect(message).toMatchObject({ from: 'person', text: 'Say hello' });
  });

  it('sums up a call in a line', () => {
    expect(argumentsSummary({ json: { command: 'cargo test\n--all' } })).toBe('cargo test');
    expect(argumentsSummary({ json: { limit: 3 } })).toBe('{"limit":3}');
    expect(argumentsSummary({ malformed: '{oops' })).toBe('{oops');
    expect(argumentsSummary(null)).toBe('');
  });

  it('reads the questions a question call asks', () => {
    const questions = questionsOf({
      json: {
        questions: [
          { question: 'Which?', options: [{ label: 'A', description: 'first' }], multiple: true },
          { question: 'Why?' },
        ],
      },
    });
    expect(questions).toEqual([
      { question: 'Which?', options: [{ label: 'A', description: 'first' }], multiple: true },
      { question: 'Why?', options: [], multiple: false },
    ]);
    expect(questionsOf({ json: {} })).toEqual([]);
  });

  it('reads the answers as the result records them, never from what the model was told', () => {
    const answers = ['Rename a → b', 'Two\nlines', ''];
    const items = threadItems([
      event('tool.call.requested', {
        call_id: 'q1',
        tool: 'question',
        arguments: {
          json: {
            questions: [{ question: 'What → where?' }, { question: 'Why?' }, { question: 'And?' }],
          },
        },
      }),
      event('tool.call.completed', {
        call_id: 'q1',
        is_error: false,
        output: [
          {
            type: 'text',
            text: 'The person answered:\n- What → where? → Rename a → b\n- Why? → Two\nlines\n- And? → (no answer)',
          },
        ],
        structured: { answers },
      }),
    ]);
    expect(items).toMatchObject([{ kind: 'question', answers }]);
    expect(answersOf(undefined)).toEqual([]);
    expect(answersOf({ answers: ['A', 7] })).toEqual(['A', '']);
  });

  it('builds the thread: messages, calls with results, approvals, questions', () => {
    const items = threadItems([
      event('thread.started', {}),
      event('message.added', { role: 'user', parts: [{ type: 'text', text: 'Fix it' }] }),
      event('tool.call.requested', {
        call_id: 'c1',
        tool: 'bash',
        arguments: { json: { command: 'rm -rf build' } },
      }),
      event('approval.requested', {
        call_id: 'c1',
        tool: 'bash',
        summary: 'bash: rm -rf build',
        ask: 'Run this command?',
        detail: 'rm -rf build',
        caution: 'It deletes build.',
      }),
      event('approval.decided', { call_id: 'c1', decision: 'deny' }),
      event('tool.call.requested', {
        call_id: 'c2',
        tool: 'read',
        arguments: { json: { path: 'a' } },
      }),
      event('tool.call.completed', {
        call_id: 'c2',
        is_error: true,
        output: [{ type: 'text', text: 'no such file' }],
      }),
      event('tool.call.requested', {
        call_id: 'c3',
        tool: 'question',
        arguments: { json: { questions: [{ question: 'Which?' }] } },
      }),
      event('tool.call.completed', {
        call_id: 'c3',
        is_error: false,
        output: [{ type: 'text', text: 'The person answered:\n- Which? → B' }],
        structured: { answers: ['B'] },
      }),
      event('message.added', {
        role: 'assistant',
        parts: [
          { type: 'reasoning', text: 'hm' },
          { type: 'text', text: 'Done' },
        ],
      }),
    ]);
    expect(items.map((item) => item.kind)).toEqual([
      'message',
      'call',
      'approval',
      'call',
      'question',
      'message',
    ]);
    expect(items[0]).toMatchObject({ from: 'person', text: 'Fix it' });
    expect(items[1]).toMatchObject({ tool: 'bash', summary: 'rm -rf build', status: 'declined' });
    expect(items[2]).toMatchObject({
      approval: 'c1',
      decision: 'deny',
      summary: 'bash: rm -rf build',
      ask: 'Run this command?',
      detail: 'rm -rf build',
      caution: 'It deletes build.',
    });
    expect(items[3]).toMatchObject({ status: 'failed', output: 'no such file' });
    expect(items[4]).toMatchObject({ call: 'c3', answers: ['B'] });
    expect(items[5]).toMatchObject({ from: 'model', text: 'Done', reasoning: 'hm' });
  });

  it('marks what the person sent while a turn ran, never what glave wrote', () => {
    const person = (text: string, author?: Record<string, unknown>): RawEvent =>
      event('message.added', {
        role: 'user',
        parts: [{ type: 'text', text }],
        ...(author === undefined ? {} : { author }),
      });
    const items = threadItems([
      event('turn.started', {}),
      person('set the port to 3000'),
      event('tool.call.requested', { call_id: 'call_1', tool: 'bash', arguments: {} }),
      event('tool.call.completed', { call_id: 'call_1', is_error: false, output: [] }),
      person('actually, 4000'),
      person('[The person cancelled this turn on purpose.]', { kind: 'glave' }),
      event('turn.ended', { outcome: 'cancelled' }),
      event('turn.started', {}),
      person('go on'),
    ]);
    const steered = items.flatMap((item) =>
      item.kind === 'message' ? [[item.text, item.steered] as const] : [],
    );
    expect(steered).toEqual([
      ['set the port to 3000', false],
      ['actually, 4000', true],
      ['[The person cancelled this turn on purpose.]', false],
      ['go on', false],
    ]);
  });

  it('draws the approval of a plan as the person’s Build it, and not glave’s prompt for it', () => {
    const glaves = (text: string): RawEvent =>
      event('message.added', {
        role: 'user',
        parts: [{ type: 'text', text }],
        author: { kind: 'glave' },
      });
    const items = threadItems([
      event('plan.approved', {}),
      event('turn.started', {}),
      glaves('The person approved this plan. Build it now.'),
      event('turn.ended', { outcome: 'cancelled' }),
      glaves('[The person cancelled this turn on purpose.]'),
    ]);
    const built = items.flatMap((item) =>
      item.kind === 'message'
        ? [[item.from, item.text.slice(0, 15), item.built === true] as const]
        : [],
    );
    expect(built).toEqual([
      ['person', '', true],
      ['person', '[The person can', false],
    ]);
  });

  it('keeps a call running and an approval waiting until they are answered', () => {
    const items = threadItems([
      event('tool.call.requested', { call_id: 'c1', tool: 'bash', arguments: { json: {} } }),
      event('approval.requested', { call_id: 'c1', tool: 'bash', summary: 'x' }),
    ]);
    expect(items[0]).toMatchObject({ status: 'running' });
    // Only the mode asks: no caution of its own. Recorded before the server said its sentence,
    // it asks with its summary.
    expect(items[1]).toMatchObject({
      decision: null,
      caution: '',
      ask: 'x',
      detail: '',
      outsideSandbox: false,
      modelReason: '',
    });
  });

  it('reads a request to run outside the sandbox, with the model’s reason apart', () => {
    const items = threadItems([
      event('approval.requested', {
        call_id: 'c1',
        tool: 'bash',
        summary: 'bash: cargo add serde',
        caution: 'It runs outside the sandbox.',
        outside_sandbox: { reason: 'cargo add writes to ~/.cargo/registry.' },
      }),
      event('approval.requested', {
        call_id: 'c2',
        tool: 'bash',
        summary: 'bash: cargo add serde',
        outside_sandbox: {},
      }),
    ]);
    expect(items[0]).toMatchObject({
      caution: 'It runs outside the sandbox.',
      outsideSandbox: true,
      modelReason: 'cargo add writes to ~/.cargo/registry.',
    });
    expect(items[1]).toMatchObject({ outsideSandbox: true, withNetwork: false, modelReason: '' });
  });

  it('reads a request to run with the network, in the sandbox otherwise', () => {
    const items = threadItems([
      event('approval.requested', {
        call_id: 'c1',
        tool: 'bash',
        summary: 'bash: npm install',
        caution: 'It can reach the internet, which commands here otherwise cannot.',
        outside_sandbox: { reason: 'npm install downloads packages.', lifted: 'network' },
      }),
    ]);
    expect(items[0]).toMatchObject({
      outsideSandbox: true,
      withNetwork: true,
      modelReason: 'npm install downloads packages.',
    });
  });

  it('grows the draft with text and reasoning only', () => {
    let draft = applyUpdate(NO_DRAFT, { kind: 'text', text: 'Hel' });
    draft = applyUpdate(draft, { kind: 'text', text: 'lo' });
    draft = applyUpdate(draft, { kind: 'reasoning', text: 'why' });
    draft = applyUpdate(draft, { kind: 'waiting' });
    expect(draft).toEqual({ text: 'Hello', reasoning: 'why' });
  });

  it('keeps the steps running now in the server’s words, from their start to their end', () => {
    let steps = stepsAfter([], { kind: 'tool_started', summary: 'bash: cargo test' });
    steps = stepsAfter(steps, { kind: 'tool_started', summary: 'read src/lock.rs' });
    steps = stepsAfter(steps, { kind: 'text', text: 'Looking.' });
    expect(steps).toEqual(['bash: cargo test', 'read src/lock.rs']);
    steps = stepsAfter(steps, {
      kind: 'tool_finished',
      summary: 'read src/lock.rs',
      is_error: false,
    });
    expect(steps).toEqual(['bash: cargo test']);
    // One that never started, as one the person declined, takes nothing away.
    steps = stepsAfter(steps, {
      kind: 'tool_declined',
      summary: 'bash: git push',
      no_one_there: false,
    });
    expect(steps).toEqual(['bash: cargo test']);
    steps = stepsAfter(steps, {
      kind: 'tool_finished',
      summary: 'bash: cargo test',
      is_error: true,
    });
    expect(steps).toEqual([]);
  });

  it('ends one step of two alike at a time', () => {
    const twice = ['read a.rs', 'read a.rs'];
    expect(
      stepsAfter(twice, { kind: 'tool_finished', summary: 'read a.rs', is_error: false }),
    ).toEqual(['read a.rs']);
  });

  it('merges events by seq, in order, once each', () => {
    const one = event('a', {}, 1);
    const two = event('b', {}, 2);
    const three = event('c', {}, 3);
    expect(mergeEvents([one, three], [two, three]).map((e) => e.seq)).toEqual([1, 2, 3]);
  });

  it('keeps the events it has when nothing is new, so nothing drawn from them is redrawn', () => {
    const events = [event('a', {}, 1), event('b', {}, 2)];
    expect(mergeEvents(events, [])).toBe(events);
    expect(mergeEvents(events, [event('b', {}, 2)])).toBe(events);
  });
});
