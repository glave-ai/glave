import type { GlzReceiptStatus } from '@glave/glaze';
import { SquareTerminal, Wrench } from 'lucide';

import type { CallItem, ThreadItem } from './conversation';
import { foldSteps, stepsSummary, toolIcon } from './steps';

function call(id: string, status: GlzReceiptStatus = 'done'): CallItem {
  return { kind: 'call', key: `call-${id}`, tool: 'bash', summary: 'ls', status, output: '' };
}

function message(id: string): ThreadItem {
  return { kind: 'message', key: id, from: 'model', text: 'ok', reasoning: '' };
}

describe('steps', () => {
  it('folds two or more calls in a row, and leaves a lone call as it is', () => {
    const folded = foldSteps([
      message('m1'),
      call('a'),
      call('b'),
      call('c'),
      message('m2'),
      call('d'),
    ]);
    expect(folded.map((item) => item.kind)).toEqual(['message', 'steps', 'message', 'call']);
    const steps = folded[1];
    expect(steps?.kind === 'steps' ? steps.calls.map((each) => each.key) : []).toEqual([
      'call-a',
      'call-b',
      'call-c',
    ]);
    expect(steps?.key).toBe('steps-call-a');
  });

  it('never folds away an approval: it ends the run', () => {
    const approval: ThreadItem = {
      kind: 'approval',
      key: 'approval-x',
      approval: 'x',
      tool: 'bash',
      summary: 'rm',
      decision: null,
    };
    const folded = foldSteps([call('a'), call('b'), approval, call('c'), call('d')]);
    expect(folded.map((item) => item.kind)).toEqual(['steps', 'approval', 'steps']);
    expect(foldSteps([])).toEqual([]);
  });

  it('sums the steps up: running, then how many failed or were declined', () => {
    expect(stepsSummary([call('a'), call('b')])).toBe('Ran 2 steps');
    expect(stepsSummary([call('a'), call('b', 'running')])).toBe('Running 2 steps');
    expect(stepsSummary([call('a', 'failed'), call('b', 'declined'), call('c')])).toBe(
      'Ran 3 steps, 1 failed, 1 declined',
    );
    expect(stepsSummary([call('a')])).toBe('Ran 1 step');
  });

  it('gives the harness’s tools their icons, and any other a wrench', () => {
    expect(toolIcon('bash')).toBe(SquareTerminal);
    expect(toolIcon('mcp__github__search')).toBe(Wrench);
  });
});
