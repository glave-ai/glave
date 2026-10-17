import type { ApprovalItem, CallItem, MessageItem, QuestionItem, ThreadItem } from './conversation';
import { turnResults } from './results';
import type { Turn } from './turns';

const ask: MessageItem = {
  kind: 'message',
  key: 'm1',
  seq: 1,
  from: 'person',
  text: 'Fix it',
  reasoning: '',
  steered: false,
};

function say(seq: number, text: string, reasoning = ''): MessageItem {
  return {
    kind: 'message',
    key: `m${String(seq)}`,
    seq,
    from: 'model',
    text,
    reasoning,
    steered: false,
  };
}

function call(seq: number, status: CallItem['status'] = 'done'): CallItem {
  return {
    kind: 'call',
    key: `c${String(seq)}`,
    seq,
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
    caution: 'It pushes to a remote.',
    decision: decided ? 'allow_once' : null,
  };
}

function question(seq: number): QuestionItem {
  return { kind: 'question', key: `q${String(seq)}`, seq, call: 'q', questions: [], answers: null };
}

function turn(items: readonly ThreadItem[], more: Partial<Turn> = {}): Turn {
  return { key: 't', items, files: [], took: null, outcome: null, cost: null, ...more };
}

describe('results', () => {
  it('puts a working turn’s reply first and folds its work behind the receipt', () => {
    const shown = turnResults(
      turn([ask, say(2, 'Looking'), call(3), approval(4, true), call(5), say(6, 'Fixed')], {
        took: 242_000,
        outcome: 'completed',
        cost: { micros: 42_000, currency: 'USD' },
      }),
      null,
      false,
    );
    expect(shown.prompt).toBe(ask);
    expect(shown.reply?.text).toBe('Fixed');
    expect(shown.before).toEqual([]);
    expect(shown.steps.map((item) => item.key)).toEqual(['m2', 'c3', 'a4', 'c5']);
    expect(shown.receipt).toEqual({ status: 'done', facts: ['4m 02s', '$0.04'], steps: '2 steps' });
    expect(shown.waiting).toEqual([]);
  });

  it('shows what waits for the person last, never folded', () => {
    const waiting = approval(4, false);
    const shown = turnResults(turn([ask, call(2), say(3, 'Ready to push'), waiting]), null, true);
    expect(shown.reply?.text).toBe('Ready to push');
    expect(shown.waiting).toEqual([waiting]);
    expect(shown.receipt?.status).toBe('waiting');
    expect(shown.receipt?.facts).toEqual([]);
  });

  it('shows a turn that did no work as what it said, in order, with no receipt', () => {
    const shown = turnResults(turn([ask, say(2, '', 'thinking'), say(3, 'Hello')]), null, false);
    expect(shown.reply?.text).toBe('Hello');
    expect(shown.before.map((item) => item.key)).toEqual(['m2']);
    expect(shown.steps).toEqual([]);
    expect(shown.receipt).toBeNull();
  });

  it('says how a turn went: failed, stopped, still going, or done', () => {
    const status = (
      more: Partial<Turn>,
      running = false,
      items: readonly ThreadItem[] = [ask, call(2)],
    ): unknown => turnResults(turn(items, more), null, running).receipt?.status;
    expect(status({ outcome: 'failed' })).toBe('failed');
    expect(status({ outcome: 'cancelled' })).toBe('stopped');
    expect(status({ outcome: 'limit_reached' })).toBe('stopped');
    expect(status({}, true)).toBe('running');
    expect(status({}, false, [ask, call(2, 'running')])).toBe('running');
    expect(status({}, false, [ask, question(2)])).toBe('waiting');
    expect(status({})).toBe('done');
    expect(turnResults(turn([ask, call(2)]), null, false).receipt?.steps).toBe('1 step');
  });

  it('puts the reply being written in the reply’s place, and the last one back with the work', () => {
    const live = { text: 'Writ', reasoning: '' };
    const shown = turnResults(turn([ask, call(2), say(3, 'Earlier')]), live, true);
    expect(shown.live).toBe(live);
    expect(shown.reply).toBeNull();
    expect(shown.steps.map((item) => item.key)).toEqual(['c2', 'm3']);
  });
});
