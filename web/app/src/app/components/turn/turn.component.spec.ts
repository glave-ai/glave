import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import type { TurnResults } from '../../lib/results';
import { CommandsService } from '../../services/commands.service';
import { PlanService } from '../../services/plan.service';
import { SourcesService } from '../../services/sources.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { TurnComponent } from './turn.component';

/** What a thread view provides, which a turn's cards reach. */
const THREAD_VIEW = [
  ThreadService,
  CommandsService,
  SourcesService,
  PlanService,
  ThreadPanelService,
];

const RESULTS: TurnResults = {
  key: 't',
  prompt: {
    kind: 'message',
    files: [],
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
    files: [],
    key: 'm9',
    seq: 9,
    from: 'model',
    text: 'Found it.',
    reasoning: '',
    steered: false,
  },
  sources: null,
  live: null,
  working: null,
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
      ask: 'Run this command?',
      detail: 'git push',
      caution: 'It pushes to a remote.',
      outsideSandbox: false,
      withNetwork: false,
      modelReason: '',
      decision: null,
    },
  ],
};

/** A turn that has just started work, with nothing of its own to show yet. */
const WORKING: TurnResults = {
  ...RESULTS,
  reply: null,
  working: { step: null, since: null },
  files: [],
  steps: [],
  receipt: null,
  waiting: [],
};

async function mount(results: TurnResults): Promise<ComponentFixture<TurnComponent>> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(new FakeSession()), ...THREAD_VIEW],
  });
  const fixture = TestBed.createComponent(TurnComponent);
  fixture.componentRef.setInput('turn', results);
  await fixture.whenStable();
  return fixture;
}

async function render(results: TurnResults): Promise<HTMLElement> {
  const fixture = await mount(results);
  return fixture.nativeElement as HTMLElement;
}

/** The working line's text, and the time beside it. */
function line(root: HTMLElement): readonly (string | undefined)[] {
  return ['thread-working-label', 'thread-working-elapsed'].map(
    (testId) => root.querySelector(`[data-testid="${testId}"]`)?.textContent,
  );
}

/** The test hooks of the blocks the turn shows, in order. */
function blocks(root: HTMLElement): readonly string[] {
  return Array.from(
    root.querySelectorAll(
      ':scope > app-thread-item > [data-testid], :scope > [data-testid], :scope > app-turn-changes > [data-testid], :scope > app-reply-sources > [data-testid]',
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

  it('shows the pages the reply drew on from the web under it', async () => {
    const root = await render({
      ...RESULTS,
      sources: {
        reply: 9,
        sentence: 'Sources: 1 page',
        pages: [{ url: 'https://tokio.rs/blog', title: 'Tokio blog' }],
      },
    });
    expect(blocks(root).slice(0, 3)).toEqual(['thread-message', 'thread-message', 'reply-sources']);
    expect(root.querySelector('[data-testid="reply-sources-sentence"]')?.textContent).toBe(
      'Sources: 1 page',
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

  describe('while it works with nothing of its own to show', () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
      vi.setSystemTime(Date.parse('2026-10-11T10:00:12.400Z'));
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('says a smithy’s word in the reply’s place, the next every couple of seconds', async () => {
      const fixture = await mount(WORKING);
      const root = fixture.nativeElement as HTMLElement;
      expect(blocks(root)).toEqual(['thread-message', 'thread-working']);
      // No time until the server records the turn's start.
      expect(line(root)).toEqual(['Forging…', undefined]);
      vi.advanceTimersByTime(2000);
      await fixture.whenStable();
      expect(line(root)).toEqual(['Honing…', undefined]);
      await expectAccessible(root);
    });

    it('shows the step a tool takes in the server’s words, and how long the turn has worked', async () => {
      const since = '2026-10-11T10:00:00.000Z';
      const fixture = await mount({
        ...WORKING,
        working: { step: 'bash: cargo test', since },
      });
      const root = fixture.nativeElement as HTMLElement;
      expect(line(root)).toEqual(['bash: cargo test', '12s']);
      vi.advanceTimersByTime(1000);
      await fixture.whenStable();
      expect(line(root)).toEqual(['bash: cargo test', '13s']);
      vi.advanceTimersByTime(60_000);
      await fixture.whenStable();
      expect(line(root)).toEqual(['bash: cargo test', '1m 13s']);
      // The step ends and the model thinks again: its words, the time going on.
      fixture.componentRef.setInput('turn', { ...WORKING, working: { step: null, since } });
      await fixture.whenStable();
      expect(line(root)).toEqual(['Forging…', '1m 13s']);
      await expectAccessible(root);
    });

    it('leaves no timer behind once the line goes, or the turn does', async () => {
      const since = '2026-10-11T10:00:00.000Z';
      const fixture = await mount({ ...WORKING, working: { step: null, since } });
      // The clock, and the words turning.
      expect(vi.getTimerCount()).toBe(2);
      fixture.componentRef.setInput('turn', RESULTS);
      await fixture.whenStable();
      const root = fixture.nativeElement as HTMLElement;
      expect(root.querySelector('[data-testid="thread-working"]')).toBeNull();
      expect(vi.getTimerCount()).toBe(0);
      fixture.componentRef.setInput('turn', { ...WORKING, working: { step: null, since } });
      await fixture.whenStable();
      expect(vi.getTimerCount()).toBe(2);
      fixture.destroy();
      expect(vi.getTimerCount()).toBe(0);
    });
  });

  it('lets a reply rise into place as it begins, and what changed settle in as it appears', async () => {
    const fixture = await mount({ ...RESULTS, reply: null, files: [], waiting: [] });
    const root = fixture.nativeElement as HTMLElement;
    fixture.componentRef.setInput('turn', {
      ...RESULTS,
      reply: null,
      live: { text: 'Writing', reasoning: '' },
      waiting: [],
    });
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thread-reply"]')?.classList).toContain('glz-rise');
    expect(root.querySelector('app-turn-changes')?.classList).toContain('glz-settle');
    // More files change: it stays as it settled, never settling again.
    fixture.componentRef.setInput('turn', {
      ...RESULTS,
      files: [...RESULTS.files, { path: 'src/main.rs', changes: [], added: 1, removed: 0 }],
    });
    await fixture.whenStable();
    expect(root.querySelector('app-turn-changes')?.classList).toContain('glz-settle');
    TestBed.resetTestingModule();
    // A turn that shows with its changes, as one read back from the log, moves nothing.
    const read = await render(RESULTS);
    expect(read.querySelector('app-turn-changes')?.classList).not.toContain('glz-settle');
  });

  it('shows reasoning open as it streams, and folds it once the answer begins', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession()), ...THREAD_VIEW],
    });
    const fixture = TestBed.createComponent(TurnComponent);
    const thinking = { ...RESULTS, reply: null, waiting: [] };
    fixture.componentRef.setInput('turn', {
      ...thinking,
      live: { text: '', reasoning: '\nThe lock is taken twice.\n\n' },
    });
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    const details = (): HTMLDetailsElement | null =>
      root.querySelector('[data-testid="thread-reasoning"] details');
    const summary = (): string | null | undefined =>
      root.querySelector('[data-testid="thread-reasoning-summary"]')?.textContent;
    expect(root.querySelector('[data-testid="thread-working"]')).toBeNull();
    expect(details()?.open).toBe(true);
    expect(summary()).toBe('Thinking…');
    // Drawn as written, but for the blank lines at its ends.
    expect(root.querySelector('[data-testid="thread-reasoning-text"]')?.textContent).toBe(
      'The lock is taken twice.',
    );
    await expectAccessible(root);
    fixture.componentRef.setInput('turn', {
      ...thinking,
      live: { text: 'Found it.', reasoning: 'The lock is taken twice.' },
    });
    await fixture.whenStable();
    expect(details()?.open).toBe(false);
    expect(summary()).toBe('Reasoning');
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
