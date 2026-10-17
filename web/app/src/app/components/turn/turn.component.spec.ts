import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import type { TurnResults } from '../../lib/results';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { TurnComponent } from './turn.component';

const RESULTS: TurnResults = {
  key: 't',
  prompt: {
    kind: 'message',
    key: 'm1',
    seq: 1,
    from: 'person',
    text: 'Fix the flaky test',
    reasoning: '',
    steered: false,
  },
  before: [],
  reply: {
    kind: 'message',
    key: 'm9',
    seq: 9,
    from: 'model',
    text: 'Found it.',
    reasoning: '',
    steered: false,
  },
  live: null,
  files: [{ path: 'src/lock.rs', changes: [], added: 16, removed: 0 }],
  steps: [
    {
      kind: 'call',
      key: 'c2',
      seq: 2,
      tool: 'bash',
      summary: 'cargo test',
      status: 'done',
      output: '',
    },
  ],
  receipt: { status: 'done', facts: ['4m 02s'], steps: '1 step' },
  waiting: [
    {
      kind: 'approval',
      key: 'a3',
      seq: 3,
      approval: 'c3',
      tool: 'bash',
      summary: 'bash: git push',
      caution: 'It pushes to a remote.',
      outsideSandbox: false,
      modelReason: '',
      decision: null,
    },
  ],
};

async function render(results: TurnResults): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(new FakeSession()), ThreadService],
  });
  const fixture = TestBed.createComponent(TurnComponent);
  fixture.componentRef.setInput('turn', results);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

/** The test hooks of the blocks the turn shows, in order. */
function blocks(root: HTMLElement): readonly string[] {
  return Array.from(
    root.querySelectorAll(
      ':scope > app-thread-item > [data-testid], :scope > [data-testid], :scope > app-turn-changes > [data-testid]',
    ),
  ).map((each) => each.getAttribute('data-testid') ?? '');
}

describe('TurnComponent', () => {
  it('shows its results first: the reply, what changed, the receipt, then what waits', async () => {
    const root = await render(RESULTS);
    expect(blocks(root)).toEqual([
      'thread-message',
      'thread-message',
      'turn-changes',
      'thread-steps',
      'thread-approval',
    ]);
    const steps = root.querySelector('[data-testid="thread-steps"]');
    expect(steps?.querySelector('details')?.open).toBe(false);
    expect(steps?.querySelectorAll('[data-testid="thread-receipt"]')).toHaveLength(1);
    expect(root.querySelector('[data-testid="thread-steps-summary"]')?.textContent).toContain(
      '1 step',
    );
    await expectAccessible(root);
  });

  it('streams the reply being written in the reply’s place', async () => {
    const root = await render({
      ...RESULTS,
      reply: null,
      live: { text: 'Writing', reasoning: '' },
      waiting: [],
    });
    const live = root.querySelector('[data-testid="thread-reply"]');
    expect(live?.getAttribute('aria-busy')).toBe('true');
    expect(live?.textContent).toContain('Writing');
  });

  it('shows a turn that did no work as its messages alone', async () => {
    const root = await render({
      ...RESULTS,
      files: [],
      steps: [],
      receipt: null,
      waiting: [],
    });
    expect(blocks(root)).toEqual(['thread-message', 'thread-message']);
  });
});
