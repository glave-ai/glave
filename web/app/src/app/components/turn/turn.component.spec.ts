import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { ChangedFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { NO_THOUGHT, type MessageItem } from '../../lib/conversation';
import type { TurnResults } from '../../lib/results';
import { ChangedFilesService } from '../../services/changed-files.service';
import { CommandsService } from '../../services/commands.service';
import { PlanService } from '../../services/plan.service';
import { SourcesService } from '../../services/sources.service';
import { StepsService } from '../../services/steps.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { TurnComponent } from './turn.component';

/** What a thread view provides, which a turn's cards reach. */
const THREAD_VIEW = [
  ThreadService,
  CommandsService,
  SourcesService,
  ChangedFilesService,
  StepsService,
  PlanService,
  ThreadPanelService,
];

const CHANGED: readonly ChangedFile[] = [
  { path: 'src/lock.rs', changes: [], added: 16, removed: 0 },
];

const REPLY: MessageItem = {
  kind: 'message',
  files: [],
  key: 'm9',
  seq: 9,
  from: 'model',
  text: 'Found it.',
  reasoning: '',
  thought: NO_THOUGHT,
  steered: false,
};

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
    thought: NO_THOUGHT,
    steered: false,
  },
  parts: [
    {
      kind: 'run',
      key: 'run-2',
      sentence: 'Ran 1 command',
      items: [
        {
          kind: 'call',
          key: 'c2',
          seq: 2,
          call: 'c2',
          tool: 'bash',
          summary: 'bash: cargo test',
          status: 'done',
          output: '',
        },
      ],
    },
    { kind: 'item', key: 'm9', item: REPLY, interim: false, sources: null },
  ],
  live: null,
  working: null,
  files: CHANGED,
  room: [],
  receipt: {
    status: 'done',
    facts: ['4m 02s', 'Claude Haiku 5.5'],
    details: ['via OpenRouter (Anthropic)'],
    steps: '1 step',
  },
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
  parts: [],
  working: { step: null, since: null },
  files: [],
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

/**
 * Shows the turn with each of `files` as the server says them, one after another, and whether
 * what changed then settles in; `undefined` while there is none.
 */
async function settles(...files: readonly TurnResults['files'][]): Promise<boolean | undefined> {
  const [first = null, ...after] = files;
  const fixture = await mount({ ...RESULTS, files: first });
  for (const each of after) {
    fixture.componentRef.setInput('turn', { ...RESULTS, files: each });
    await fixture.whenStable();
  }
  const root = fixture.nativeElement as HTMLElement;
  return root.querySelector('app-turn-changes')?.classList.contains('glz-settle');
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
      [
        ':scope > app-thread-item > [data-testid]',
        ':scope > [data-testid]',
        ':scope > app-turn-changes > [data-testid]',
        ':scope > app-turn-part > [data-testid]',
        ':scope > app-turn-part > app-thread-item > [data-testid]',
        ':scope > app-turn-part > app-reply-sources > [data-testid]',
      ].join(', '),
    ),
  ).map((each) => each.getAttribute('data-testid') ?? '');
}

describe('TurnComponent', () => {
  it('shows its work in order: each run in the server’s line, the reply, what changed, the receipt, then what waits', async () => {
    const root = await render(RESULTS);
    expect(blocks(root)).toEqual([
      'thread-message',
      'thread-run',
      'thread-message',
      'turn-changes',
      'thread-steps',
      'thread-approval',
    ]);
    const run = root.querySelector('[data-testid="thread-run"]');
    expect(run?.querySelector('details')?.open).toBe(false);
    expect(root.querySelector('[data-testid="thread-run-summary"]')?.textContent).toContain(
      'Ran 1 command',
    );
    expect(run?.querySelectorAll('[data-testid="thread-receipt"]')).toHaveLength(1);
    const receipt = root.querySelector('[data-testid="thread-steps"]');
    expect(receipt?.querySelector('details')?.open).toBe(false);
    expect(root.querySelector('[data-testid="thread-steps-summary"]')?.textContent).toContain(
      'Claude Haiku 5.5',
    );
    expect(root.querySelector('[data-testid="thread-steps-detail"]')?.textContent).toBe(
      'via OpenRouter (Anthropic)',
    );
    await expectAccessible(root);
  });

  it('names a message said along the way an update, and the reply a reply', async () => {
    const root = await render({
      ...RESULTS,
      parts: [
        {
          kind: 'item',
          key: 'm5',
          item: { ...REPLY, key: 'm5', seq: 5 },
          interim: true,
          sources: null,
        },
        ...RESULTS.parts,
      ],
    });
    const names = Array.from(root.querySelectorAll('[data-testid="thread-message"]')).map((each) =>
      each.getAttribute('aria-label'),
    );
    expect(names).toEqual(['You', 'Update', 'Reply']);
  });

  it('shows the pages the reply drew on from the web under it', async () => {
    const sources = {
      reply: 9,
      sentence: 'Sources: 1 page',
      pages: [{ url: 'https://tokio.rs/blog', title: 'Tokio blog' }],
    };
    const root = await render({
      ...RESULTS,
      parts: [{ kind: 'item', key: 'm9', item: REPLY, interim: false, sources }],
    });
    expect(blocks(root).slice(0, 3)).toEqual(['thread-message', 'thread-message', 'reply-sources']);
    expect(root.querySelector('[data-testid="reply-sources-sentence"]')?.textContent).toBe(
      'Sources: 1 page',
    );
    await expectAccessible(root);
  });

  it('streams what the model writes now after what it said before', async () => {
    const root = await render({
      ...RESULTS,
      live: { text: 'Writing', reasoning: '', thought: NO_THOUGHT },
      waiting: [],
    });
    expect(blocks(root).slice(0, 4)).toEqual([
      'thread-message',
      'thread-run',
      'thread-message',
      'thread-reply',
    ]);
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
    const fixture = await mount({ ...RESULTS, parts: [], files: [], waiting: [] });
    const root = fixture.nativeElement as HTMLElement;
    fixture.componentRef.setInput('turn', {
      ...RESULTS,
      parts: [],
      live: { text: 'Writing', reasoning: '', thought: NO_THOUGHT },
      waiting: [],
    });
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thread-reply"]')?.classList).toContain('glz-rise');
    expect(root.querySelector('app-turn-changes')?.classList).toContain('glz-settle');
    // More files change: it stays as it settled, never settling again.
    fixture.componentRef.setInput('turn', {
      ...RESULTS,
      files: [...CHANGED, { path: 'src/main.rs', changes: [], added: 1, removed: 0 }],
    });
    await fixture.whenStable();
    expect(root.querySelector('app-turn-changes')?.classList).toContain('glz-settle');
    TestBed.resetTestingModule();
    // A turn that shows with its changes, as one read back from the log, moves nothing.
    const read = await render(RESULTS);
    expect(read.querySelector('app-turn-changes')?.classList).not.toContain('glz-settle');
  });

  it('lets a card that needs the person rise into place as it appears, and only then', async () => {
    const fixture = await mount({ ...RESULTS, waiting: [] });
    const root = fixture.nativeElement as HTMLElement;
    fixture.componentRef.setInput('turn', RESULTS);
    await fixture.whenStable();
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.classList).toContain('glz-rise');
    await expectAccessible(root);
    TestBed.resetTestingModule();
    // A turn that shows with it waiting, as one read back from the log, moves nothing.
    const read = await render(RESULTS);
    expect(read.querySelector('[data-testid="thread-approval"]')?.classList).not.toContain(
      'glz-rise',
    );
  });

  it('lets nothing rise that the thread’s history brings as it opens', async () => {
    let read: (events: unknown) => void = () => undefined;
    const session = new FakeSession().answer(
      'thread/events',
      () =>
        new Promise((resolve) => {
          read = resolve;
        }),
    );
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(session), ...THREAD_VIEW],
    });
    const opened = TestBed.inject(ThreadService).open('thread-1');
    const fixture = TestBed.createComponent(TurnComponent);
    fixture.componentRef.setInput('turn', { ...RESULTS, waiting: [] });
    await fixture.whenStable();
    // The next page of its history holds the approval it waits on.
    fixture.componentRef.setInput('turn', RESULTS);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="thread-approval"]')?.classList).not.toContain(
      'glz-rise',
    );
    read({ events: [], next: null });
    await opened;
  });

  it('settles in nothing the thread had changed as it opened, though said only after', async () => {
    // Not known as the thread opens, then said: it was there before the person looked.
    expect(await settles(null, CHANGED)).toBe(false);
    TestBed.resetTestingModule();
    // Said to be nothing as it opened, then changed: it changed as the person watched.
    expect(await settles(null, [], CHANGED)).toBe(true);
  });

  it('keeps the room of what it changed, unseen, until that is read, then shows it there', async () => {
    const room = [{ path: 'src/lock.rs', changes: [], added: 0, removed: 0 }];
    const fixture = await mount({ ...RESULTS, files: null, room });
    const root = fixture.nativeElement as HTMLElement;
    const kept = root.querySelector('app-turn-changes.room');
    expect(kept?.querySelector('[data-testid="turn-changes-room"]')).not.toBeNull();
    expect(getComputedStyle(kept ?? root).visibility).toBe('hidden');
    expect(root.querySelector('[data-testid="turn-changes"]')).toBeNull();
    await expectAccessible(root);
    fixture.componentRef.setInput('turn', { ...RESULTS, room: [] });
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="turn-changes-room"]')).toBeNull();
    expect(root.querySelector('[data-testid="turn-changes"]')).not.toBeNull();
  });

  it('shows the reasoning as it streams in one line: what it thinks now, then how long it took', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession()), ...THREAD_VIEW],
    });
    const fixture = TestBed.createComponent(TurnComponent);
    const thinking = { ...RESULTS, parts: [], waiting: [] };
    fixture.componentRef.setInput('turn', {
      ...thinking,
      live: {
        text: '',
        reasoning: 'The lock is taken twice.',
        thought: { headline: 'The lock is taken twice.', ms: null },
      },
    });
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    const line = (): HTMLElement | null =>
      root.querySelector('[data-testid="thread-thought-line"]');
    const label = (): string | null | undefined =>
      root.querySelector('[data-testid="thread-thought-label"]')?.textContent;
    expect(root.querySelector('[data-testid="thread-working"]')).toBeNull();
    expect(label()).toBe('Thinking');
    expect(line()?.classList).toContain('live');
    expect(root.querySelector('[data-testid="thread-thought-headline"]')?.textContent).toBe(
      'The lock is taken twice.',
    );
    // Nothing to open while the model thinks.
    expect(root.querySelector('[data-testid="thread-thought-text"]')).toBeNull();
    await expectAccessible(root);
    fixture.componentRef.setInput('turn', {
      ...thinking,
      live: {
        text: 'Found it.',
        reasoning: '\nThe lock is taken twice.\n\n',
        thought: { headline: 'The lock is taken twice.', ms: 3000 },
      },
    });
    await fixture.whenStable();
    expect(label()).toBe('Thought for 3s');
    expect(line()?.classList).not.toContain('live');
    // Done, it opens to the reasoning in full, but for the blank lines at its ends.
    line()?.click();
    await fixture.whenStable();
    expect(line()?.getAttribute('aria-expanded')).toBe('true');
    expect(root.querySelector('[data-testid="thread-thought-text"]')?.textContent).toBe(
      'The lock is taken twice.',
    );
    await expectAccessible(root);
  });

  it('shows a turn that did no work as its messages alone', async () => {
    const root = await render({
      ...RESULTS,
      parts: [{ kind: 'item', key: 'm9', item: REPLY, interim: false, sources: null }],
      files: [],
      receipt: null,
      waiting: [],
    });
    expect(blocks(root)).toEqual(['thread-message', 'thread-message']);
  });
});
