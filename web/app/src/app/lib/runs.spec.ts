import type { CallItem } from './conversation';
import { stepping, stepsOf } from './runs';
import type { Turn } from './turns';

describe('runs', () => {
  it('finds each call’s run as the server told it, and the replies', () => {
    const steps = stepsOf({
      runs: [
        { first: 3, calls: ['a', 'b'], sentence: 'Ran 2 commands' },
        { first: 7, calls: ['c'], sentence: 'Read 1 file' },
      ],
      replies: [9],
    });
    expect(steps.runs.get('b')).toEqual({ key: 'run-3', sentence: 'Ran 2 commands' });
    expect(steps.runs.get('c')).toEqual({ key: 'run-7', sentence: 'Read 1 file' });
    expect([...steps.replies]).toEqual([9]);
  });

  it('asks again as calls are asked for and end, and as a turn ends, once there is any call', () => {
    const turn = (items: Turn['items'], ended = false): Turn => ({
      key: 't',
      items,
      startSeq: 1,
      took: null,
      started: null,
      model: null,
      result: ended ? { outcome: 'completed' } : null,
    });
    const call = (status: CallItem['status']): CallItem => ({
      kind: 'call',
      key: 'call-a',
      seq: 2,
      call: 'a',
      tool: 'bash',
      summary: 'bash: ls',
      status,
      output: '',
    });
    expect(stepping([turn([], true)])).toBeNull();
    const running = stepping([turn([call('running')])]);
    const done = stepping([turn([call('done')])]);
    const ended = stepping([turn([call('done')], true)]);
    expect(new Set([running, done, ended]).size).toBe(3);
    expect(running).not.toBeNull();
  });
});
