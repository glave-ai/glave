import type { RawEvent } from '@glave/client';

import {
  NO_DRAFT,
  NO_THOUGHT,
  answersOf,
  applyUpdate,
  mergeEvents,
  partsText,
  promptOf,
  questionsOf,
  serviceDetailOf,
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

  it("says each step in the server's line, never one of its own from the arguments", () => {
    const items = threadItems([
      event('tool.call.requested', {
        call_id: 'c1',
        tool: 'mcp__search__web_search_exa',
        arguments: { json: { query: 'tokio', numResults: 5 } },
        summary: 'Searched the web: tokio',
      }),
      // A call recorded before the server said its line: its tool alone.
      event('tool.call.requested', {
        call_id: 'c2',
        tool: 'bash',
        arguments: { json: { command: 'cargo test' } },
      }),
    ]);
    expect(items).toMatchObject([
      { kind: 'call', tool: 'mcp__search__web_search_exa', summary: 'Searched the web: tokio' },
      { kind: 'call', tool: 'bash', summary: 'bash' },
    ]);
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
        summary: 'bash: rm -rf build',
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
    expect(items[1]).toMatchObject({
      tool: 'bash',
      summary: 'bash: rm -rf build',
      status: 'declined',
    });
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

  it('reads what a reply’s reasoning came to as the server recorded it, and nothing of an older one', () => {
    const items = threadItems([
      event('message.added', {
        role: 'assistant',
        parts: [{ type: 'reasoning', text: 'Done.' }],
        thought: { headline: 'Done.', ms: 4250 },
      }),
      event('message.added', {
        role: 'assistant',
        parts: [{ type: 'reasoning', text: 'Old.' }],
      }),
      event('message.added', {
        role: 'assistant',
        parts: [{ type: 'reasoning', text: 'x' }],
        thought: { headline: '', ms: 'soon' },
      }),
    ]);
    expect(items.map((item) => (item.kind === 'message' ? item.thought : null))).toEqual([
      { headline: 'Done.', ms: 4250 },
      NO_THOUGHT,
      NO_THOUGHT,
    ]);
  });

  it('grows the draft with text and reasoning, and starts it afresh as a model call begins', () => {
    let draft = applyUpdate(NO_DRAFT, { kind: 'reasoning', text: 'why' });
    // What the reasoning came to as the server says it: its headline, then how long it took.
    draft = applyUpdate(draft, { kind: 'thinking', headline: 'Checking the port.' });
    expect(draft.thought).toEqual({ headline: 'Checking the port.', ms: null });
    draft = applyUpdate(draft, { kind: 'thought', ms: 4250 });
    draft = applyUpdate(draft, { kind: 'text', text: 'Hel' });
    draft = applyUpdate(draft, { kind: 'text', text: 'lo' });
    draft = applyUpdate(draft, { kind: 'tool_started', summary: 'bash: ls' });
    expect(draft).toEqual({
      text: 'Hello',
      reasoning: 'why',
      thought: { headline: 'Checking the port.', ms: 4250 },
    });
    // A call tried again: what it had written goes.
    expect(applyUpdate(draft, { kind: 'waiting' })).toBe(NO_DRAFT);
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

describe('what a service asks', () => {
  const asked = (seq: number, call: string): RawEvent =>
    ({
      seq,
      type: 'service.asked',
      data: {
        call_id: call,
        service: 'GitHub',
        ask: 'GitHub asks you, not the model',
        note: 'Your answer goes only to GitHub.',
        message: 'Which label?',
        state: { sha256: 'x', bytes: 1, media_type: 'application/json' },
        fields: [
          {
            name: 'label',
            label: 'Label',
            required: true,
            kind: {
              type: 'choice',
              options: [{ value: 'bug', label: 'Bug' }],
              multiple: true,
              default: ['bug'],
            },
          },
          {
            name: 'note',
            label: 'Note',
            description: 'More',
            kind: { type: 'text', default: 'x' },
          },
          { name: 'count', label: 'Count', kind: { type: 'number', integer: true, default: 2 } },
          { name: 'sure', label: 'Sure', kind: { type: 'yes_no', default: false } },
        ],
      },
    }) as unknown as RawEvent;
  const answered = (seq: number, call: string, told: string): RawEvent =>
    ({
      seq,
      type: 'service.answered',
      data: { call_id: call, service: 'GitHub', outcome: 'answered', told },
    }) as unknown as RawEvent;

  it('reads who asks, what, and its fields, as they were recorded', () => {
    expect(threadItems([asked(4, 'c1')])).toEqual([
      {
        kind: 'service',
        key: 'service-4',
        seq: 4,
        call: 'c1',
        service: 'GitHub',
        ask: 'GitHub asks you, not the model',
        note: 'Your answer goes only to GitHub.',
        message: 'Which label?',
        detail: null,
        told: null,
        fields: [
          {
            name: 'label',
            label: 'Label',
            description: '',
            required: true,
            kind: {
              type: 'choice',
              options: [{ value: 'bug', label: 'Bug' }],
              multiple: true,
              default: ['bug'],
            },
          },
          {
            name: 'note',
            label: 'Note',
            description: 'More',
            required: false,
            kind: { type: 'text', default: 'x' },
          },
          {
            name: 'count',
            label: 'Count',
            description: '',
            required: false,
            kind: { type: 'number', integer: true, default: 2 },
          },
          {
            name: 'sure',
            label: 'Sure',
            description: '',
            required: false,
            kind: { type: 'yes_no', default: false },
          },
        ],
      },
    ]);
  });

  it('reads what glave adds as the server recorded it', () => {
    expect(
      serviceDetailOf({
        front: 'name: tidy',
        text: '# Tidy',
        files: [
          { path: 'scripts/tidy.sh', text: '#!/bin/sh', runs_code: true, cut: true },
          { path: 'notes.md', text: 'n' },
        ],
        cut: 'Too long to show in full here: 1 part cut short.',
      }),
    ).toEqual({
      front: 'name: tidy',
      text: '# Tidy',
      files: [
        { path: 'scripts/tidy.sh', text: '#!/bin/sh', runsCode: true },
        { path: 'notes.md', text: 'n', runsCode: false },
      ],
      cut: 'Too long to show in full here: 1 part cut short.',
    });
    expect(serviceDetailOf(undefined)).toBeNull();
  });

  it('keeps each thing a call asks apart, each with what came of it', () => {
    const items = threadItems([
      asked(4, 'c1'),
      answered(5, 'c1', 'You answered GitHub'),
      asked(6, 'c1'),
    ]);
    expect(items.map((item) => (item.kind === 'service' ? [item.key, item.told] : null))).toEqual([
      ['service-4', 'You answered GitHub'],
      ['service-6', null],
    ]);
  });
});
