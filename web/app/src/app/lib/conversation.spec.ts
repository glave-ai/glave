import type { RawEvent } from '@glave/client';

import {
  NO_DRAFT,
  answersFrom,
  applyUpdate,
  argumentsSummary,
  mergeEvents,
  partsText,
  promptOf,
  questionsOf,
  threadItems,
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

  it('reads the answers back from what the model was told', () => {
    expect(answersFrom('The person answered:\n- Which? → A\n- Why? → (no answer)')).toEqual([
      'A',
      '',
    ]);
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
      event('approval.requested', { call_id: 'c1', tool: 'bash', summary: 'Delete build' }),
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
    expect(items[2]).toMatchObject({ approval: 'c1', decision: 'deny', summary: 'Delete build' });
    expect(items[3]).toMatchObject({ status: 'failed', output: 'no such file' });
    expect(items[4]).toMatchObject({ call: 'c3', answers: ['B'] });
    expect(items[5]).toMatchObject({ from: 'model', text: 'Done', reasoning: 'hm' });
  });

  it('keeps a call running and an approval waiting until they are answered', () => {
    const items = threadItems([
      event('tool.call.requested', { call_id: 'c1', tool: 'bash', arguments: { json: {} } }),
      event('approval.requested', { call_id: 'c1', tool: 'bash', summary: 'x' }),
    ]);
    expect(items[0]).toMatchObject({ status: 'running' });
    expect(items[1]).toMatchObject({ decision: null });
  });

  it('grows the draft with text and reasoning only', () => {
    let draft = applyUpdate(NO_DRAFT, { kind: 'text', text: 'Hel' });
    draft = applyUpdate(draft, { kind: 'text', text: 'lo' });
    draft = applyUpdate(draft, { kind: 'reasoning', text: 'why' });
    draft = applyUpdate(draft, { kind: 'waiting' });
    expect(draft).toEqual({ text: 'Hello', reasoning: 'why' });
  });

  it('merges events by seq, in order, once each', () => {
    const one = event('a', {}, 1);
    const two = event('b', {}, 2);
    const three = event('c', {}, 3);
    expect(mergeEvents([one, three], [two, three]).map((e) => e.seq)).toEqual([1, 2, 3]);
  });
});
