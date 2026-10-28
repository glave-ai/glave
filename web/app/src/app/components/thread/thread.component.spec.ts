import { LiveAnnouncer } from '@angular/cdk/a11y';
import { TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import type { Notification } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ChangesService } from '../../services/changes.service';
import { PanelService } from '../../services/panel.service';
import { FakeSession, FakeTurn } from '../../testing/fake-session';
import { MODELS, SERVER, changed, editedFile, event, record, thread } from '../../testing/fixtures';
import { openThread as open } from '../../testing/thread-view';
import type { TurnResults } from '../../lib/results';
import { TurnComponent } from '../turn/turn.component';

const HISTORY = [
  event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Clean up' }] }),
  event(2, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'bash',
    arguments: { json: { command: 'rm -rf build' } },
    summary: 'bash: rm -rf build',
  }),
  event(3, 'approval.requested', { call_id: 'c1', tool: 'bash', summary: 'Delete build/' }),
  event(4, 'tool.call.requested', {
    call_id: 'c2',
    tool: 'question',
    arguments: { json: { questions: [{ question: 'Keep logs?', options: [{ label: 'Yes' }] }] } },
  }),
];

/** A turn that edited a file: a thread with something for its panel. */
const EDITED = [
  event(1, 'turn.started', {}),
  event(2, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Fix the typo' }] }),
  event(3, 'tool.call.requested', { call_id: 'c1', tool: 'edit', arguments: { json: {} } }),
  event(4, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
  event(5, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Fixed.' }] }),
  event(6, 'turn.ended', { outcome: 'completed' }),
];

/** What the server says that turn changed. */
const README = changed(1, [editedFile('README.md', ['teh'], ['the'])]);

const ANSWERED = [
  event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Hi' }] }),
  event(2, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Hello' }] }),
  event(3, 'model.responded', { model: { provider: 'gateway', name: 'hope-12b' } }),
];

function session(events: readonly unknown[], changes = changed(1, [])): FakeSession {
  return new FakeSession()
    .answer('thread/events', { events })
    .answer('thread/changes', changes)
    .answer('model/list', { models: MODELS })
    .answer('server/discover', SERVER)
    .answer('schedule/list', { schedules: [] })
    .answer('tasks/list', { tasks: [] })
    .answer('project/list', { projects: [record('p1', 'glave')] })
    .answer('thread/list', { threads: [thread('thread-1', 'Tidy the repo', { project: 'p1' })] });
}

const settle = (): Promise<unknown> => new Promise((resolve) => setTimeout(resolve));

async function type(root: HTMLElement, text: string): Promise<void> {
  const input = root.querySelector<HTMLTextAreaElement>('[data-testid="thread-composer-input"]');
  if (input) {
    input.value = text;
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
  }
  await settle();
}

describe('ThreadComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    document.body.click();
  });

  it('shows a globe in the composer while web search is on', async () => {
    const off = await open(session(ANSWERED));
    expect(off.root.querySelector('[data-testid="thread-searching"]')).toBeNull();
    TestBed.resetTestingModule();
    const on = session(ANSWERED).answer('search/get', {
      search: { kind: 'chosen', on: true, detail: 'Uses Exa, free. Your searches go to Exa.' },
    });
    const { root } = await open(on);
    const globe = root.querySelector('[data-testid="thread-searching"]');
    expect(globe?.getAttribute('aria-label')).toBe('Web search is on');
    expect(globe?.closest('[data-testid="thread-composer-box"]')).not.toBeNull();
    await expectAccessible(root);
  });

  it('shows the top bar, and the conversation with its receipts, approvals and questions', async () => {
    const fake = session(HISTORY);
    const { root } = await open(fake);
    expect(root.querySelector('[data-testid="thread-title"]')?.textContent.trim()).toBe(
      'Tidy the repo',
    );
    expect(root.querySelector('[data-testid="thread-meta"]')?.textContent.trim()).toBe('glave');
    expect(root.querySelector('[data-testid="thread-message"]')?.textContent).toContain('Clean up');
    expect(root.querySelector('[data-testid="thread-receipt"]')?.textContent).toContain(
      'rm -rf build',
    );
    expect(root.querySelector('[data-testid="thread-question"]')?.textContent).toContain(
      'Keep logs?',
    );
    // What the agent may do and the model in the composer's quiet line.
    const status = root.querySelector('[data-testid="thread-composer-status"]');
    expect(status?.querySelector('[data-testid="permissions"]')?.textContent).toContain('Auto');
    expect(status?.querySelector('[data-testid="model-picker"]')).not.toBeNull();
    // What waits for the person comes last, after the receipt.
    const order = Array.from(
      root.querySelectorAll(
        '[data-testid="thread-steps"], [data-testid="thread-approval"], [data-testid="thread-question"]',
      ),
    ).map((each) => each.getAttribute('data-testid'));
    expect(order).toEqual(['thread-steps', 'thread-approval', 'thread-question']);
    expect(
      root.querySelector('[data-testid="thread-steps-summary"]')?.textContent.replace(/\s+/gu, ' '),
    ).toContain('waiting');
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-approval-allow-once"]')?.click();
    await settle();
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c1', decision: 'allow_once' },
    ]);
  });

  it('shows what the project asks to be trusted last, sends the answer, then rests it', async () => {
    const ask = {
      ask: 'ask-1',
      items: [
        { text: 'Runs a tool server', detail: 'npx @acme/mcp' },
        { text: 'Lets glave work outside the sandbox without asking' },
      ],
    };
    const fake = session(ANSWERED)
      .answer('thread/get', { thread: thread('thread-1', 'Tidy the repo', { trust: ask }) })
      .answer('trust/answer', { thread: thread('thread-1', 'Tidy the repo') });
    const { root, fixture } = await open(fake);
    const card = root.querySelector('[data-testid="thread-trust"]');
    expect(card?.textContent).toContain('This project brings its own tools and settings');
    expect(card?.textContent).not.toMatch(/\.mcp\.json|config\.toml|trust\.toml/u);
    const items = Array.from(root.querySelectorAll('[data-testid="thread-trust-item"]'));
    expect(items.map((item) => item.textContent.replace(/\s+/gu, ' ').trim())).toEqual([
      'Runs a tool server npx @acme/mcp',
      'Lets glave work outside the sandbox without asking',
    ]);
    const last = root.querySelector('[data-testid="thread-items"] .reading')?.lastElementChild;
    expect(last?.getAttribute('data-testid')).toBe('thread-trust');
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-trust-trust"]')?.click();
    await settle();
    await fixture.whenStable();
    expect(fake.paramsOf('trust/answer')).toEqual([
      { thread: 'thread-1', ask: 'ask-1', answer: 'trust' },
    ]);
    expect(root.querySelector('[data-testid="thread-trust-answered"]')?.textContent).toBe(
      'Trusted',
    );
    expect(root.querySelector('[data-testid="thread-trust-trust"]')).toBeNull();
    await expectAccessible(root);
  });

  it('names no model over a reply, and gives only the reply a copy button', async () => {
    const { root } = await open(session(ANSWERED));
    const messages = root.querySelectorAll('[data-testid="thread-message"]');
    expect(messages[0]?.getAttribute('aria-label')).toBe('You');
    expect(messages[1]?.getAttribute('aria-label')).toBe('Reply');
    expect(root.querySelector('glz-avatar')).toBeNull();
    expect(root.querySelector('[data-testid="thread-items"] .author')).toBeNull();
    expect(root.querySelector('[data-testid="thread-items"]')?.textContent).not.toContain('Hope');
    expect(messages[0]?.querySelector('app-message-text')?.textContent).toContain('Hi');
    expect(messages[0]?.querySelector('[data-testid="message-copy"]')).toBeNull();
    expect(messages[1]?.querySelector('.actions [data-testid="message-copy"]')).not.toBeNull();
    await expectAccessible(root);
  });

  it('floats "Jump to latest" over the conversation once scrolled up, and jumps back', async () => {
    const { root, fixture } = await open(session(ANSWERED));
    const items = root.querySelector<HTMLElement>('[data-testid="thread-items"]');
    const latest = root.querySelector('[data-testid="thread-latest"]');
    // It lives in the layer over the conversation, never in the dock's own flow.
    expect(latest?.parentElement?.classList).toContain('float');
    expect(latest?.classList).not.toContain('shown');
    if (items === null) {
      throw new Error('no conversation');
    }
    Object.defineProperty(items, 'scrollHeight', { value: 1000 });
    Object.defineProperty(items, 'clientHeight', { value: 200 });
    items.scrollTop = 0;
    items.dispatchEvent(new Event('scroll'));
    await fixture.whenStable();
    expect(latest?.classList).toContain('shown');
    expect(latest?.querySelector('button')?.textContent.trim()).toBe('Jump to latest');
    await expectAccessible(root);
    latest?.querySelector('button')?.click();
    await fixture.whenStable();
    expect(latest?.classList).not.toContain('shown');
  });

  it('never shows "Jump to latest" between a press and its release, which would take the click', async () => {
    const { root, fixture } = await open(session(ANSWERED));
    const items = root.querySelector<HTMLElement>('[data-testid="thread-items"]');
    const latest = root.querySelector('[data-testid="thread-latest"]');
    if (items === null) {
      throw new Error('no conversation');
    }
    Object.defineProperty(items, 'scrollHeight', { value: 1000 });
    Object.defineProperty(items, 'clientHeight', { value: 200 });
    // Pressed on a line a scroll has just brought into view, away from the bottom.
    document.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    items.scrollTop = 760;
    items.dispatchEvent(new Event('scroll'));
    await fixture.whenStable();
    expect(latest?.classList).not.toContain('shown');
    // Released: the click has gone where the press began; now it shows.
    document.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    await fixture.whenStable();
    expect(latest?.classList).toContain('shown');

    // Held shown too, so a press on it ends on it.
    document.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    items.scrollTop = 800;
    items.dispatchEvent(new Event('scroll'));
    await fixture.whenStable();
    expect(latest?.classList).toContain('shown');
    document.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true }));
    await fixture.whenStable();
    expect(latest?.classList).not.toContain('shown');
  });

  it('holds "Jump to latest" for no press a menu or a drag takes over', async () => {
    const { root, fixture } = await open(session(ANSWERED));
    const items = root.querySelector<HTMLElement>('[data-testid="thread-items"]');
    const latest = root.querySelector('[data-testid="thread-latest"]');
    if (items === null) {
      throw new Error('no conversation');
    }
    Object.defineProperty(items, 'scrollHeight', { value: 1000 });
    Object.defineProperty(items, 'clientHeight', { value: 200 });
    const scrolled = async (top: number): Promise<void> => {
      items.scrollTop = top;
      items.dispatchEvent(new Event('scroll'));
      await fixture.whenStable();
    };
    // The other button opens a menu, which takes the release: nothing is held.
    document.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 2 }));
    await scrolled(0);
    expect(latest?.classList).toContain('shown');
    await scrolled(800);
    // A press that opens a menu (Control and click on a Mac) or starts a drag ends there.
    for (const taken of ['contextmenu', 'dragstart']) {
      document.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      document.dispatchEvent(new Event(taken, { bubbles: true }));
      await scrolled(0);
      expect(latest?.classList).toContain('shown');
      await scrolled(800);
      expect(latest?.classList).not.toContain('shown');
    }
  });

  it('shows a turn that worked results first: the reply, what changed, then its steps', async () => {
    const call = (seq: number, id: string, tool: string, json: object, summary: string): unknown =>
      event(seq, 'tool.call.requested', { call_id: id, tool, arguments: { json }, summary });
    const done = (seq: number, id: string): unknown =>
      event(seq, 'tool.call.completed', { call_id: id, is_error: false, output: [] });
    const events = [
      event(1, 'turn.started', {}),
      event(2, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Fix it' }] }),
      call(3, 'a', 'bash', { command: 'cargo test' }, 'bash: cargo test'),
      done(4, 'a'),
      call(5, 'b', 'edit', { path: 'src/parse.rs' }, 'edit src/parse.rs'),
      done(6, 'b'),
      call(7, 'c', 'write', { path: 'tests/empty.rs' }, 'write tests/empty.rs (0 lines)'),
      done(8, 'c'),
      event(9, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Done' }] }),
    ];
    const both = [editedFile('src/parse.rs', [], ['x']), editedFile('tests/empty.rs', [], ['x'])];
    const { root, fixture } = await open(session(events, changed(1, both)));
    const order = Array.from(
      root.querySelectorAll(
        '[data-testid="thread-message"], [data-testid="turn-changes"], [data-testid="thread-steps"]',
      ),
    ).map((each) => each.getAttribute('data-testid'));
    expect(order).toEqual(['thread-message', 'thread-message', 'turn-changes', 'thread-steps']);
    expect(root.querySelector('[data-testid="turn-changes-count"]')?.textContent).toBe('2 files');
    // There before the thread opened, though read just after: it shows still, never settling in.
    expect(root.querySelector('app-turn-changes')?.classList).not.toContain('glz-settle');
    // And in the thread's panel, for the whole thread.
    expect(root.querySelectorAll('[data-testid="panel-changes-file"]')).toHaveLength(2);
    const steps = root.querySelector('[data-testid="thread-steps"]');
    const summary = root.querySelector<HTMLElement>('[data-testid="thread-steps-summary"]');
    expect(summary?.querySelector('.state')?.textContent).toBe('Done');
    expect(summary?.querySelector('.count')?.textContent).toBe('3 steps');
    expect(steps?.querySelector('details')?.open).toBe(false);
    const receipts = Array.from(steps?.querySelectorAll('[data-testid="thread-receipt"]') ?? []);
    // Each step in the server's line.
    expect(receipts.map((receipt) => receipt.querySelector('.what')?.textContent)).toEqual([
      'bash: cargo test',
      'edit src/parse.rs',
      'write tests/empty.rs (0 lines)',
    ]);
    expect(receipts.every((receipt) => receipt.querySelector('.kind svg') !== null)).toBe(true);
    summary?.click();
    await fixture.whenStable();
    expect(steps?.querySelector('details')?.open).toBe(true);
    await expectAccessible(root);
  });

  it('sends from the composer and shows the reply streaming in', async () => {
    const fake = session([]).streams([
      {
        jsonrpc: '2.0',
        method: 'turn/live',
        params: { thread: 'thread-1', update: { kind: 'text', text: 'Working on it' } },
      },
      {
        jsonrpc: '2.0',
        method: 'thread/event',
        params: {
          thread: 'thread-1',
          event: event(1, 'message.added', {
            role: 'assistant',
            parts: [{ type: 'text', text: 'Done.' }],
          }),
        },
      },
    ]);
    const { root } = await open(fake);
    await type(root, 'Go');
    expect(fake.paramsOf('turn/start')).toMatchObject([{ thread: 'thread-1', prompt: 'Go' }]);
    expect(root.textContent).toContain('Done.');
  });

  it('streams a reply into the last turn alone, never drawing the turns before it again', async () => {
    const turn = new FakeTurn();
    const fake = session(ANSWERED).turn(turn);
    const { root, fixture } = await open(fake);
    await type(root, 'And now?');
    const drawn = (): readonly TurnResults[] =>
      fixture.debugElement
        .queryAll(By.directive(TurnComponent))
        .map((each) => (each.componentInstance as TurnComponent).turn());
    const word = (text: string): Notification => ({
      jsonrpc: '2.0',
      method: 'turn/live',
      params: { thread: 'thread-1', update: { kind: 'text', text } },
    });
    turn.send(word('Look'));
    await settle();
    await fixture.whenStable();
    const before = drawn();
    turn.send(word('ing'));
    await settle();
    await fixture.whenStable();
    const after = drawn();
    expect(after).toHaveLength(2);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
    expect(root.textContent).toContain('Looking');
    await expectAccessible(root);
    turn.end();
  });

  it('shows the first message the new chat sent, and sends it again when no answer came', async () => {
    const lost = new FakeTurn();
    lost.fail(new TypeError('Failed to fetch'));
    const fake = session([]).turn(lost).streams([]);
    const { root, fixture } = await open(fake, { thread: 'thread-1', prompt: 'Plan' });
    expect(root.querySelector('[data-testid="thread-message"]')?.textContent).toContain('Plan');
    expect(root.querySelector('[data-testid="thread-lost"]')?.textContent).toContain(
      'This message may not have reached glave.',
    );
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-send-again"]')?.click();
    await settle();
    await fixture.whenStable();
    const [first, again] = fake.paramsOf('turn/start');
    expect(again).toEqual(first);
    expect(root.querySelector('[data-testid="thread-lost"]')).toBeNull();
  });

  it('takes a message while a turn runs: waiting to be read, then read where the model saw it', async () => {
    const turn = new FakeTurn();
    const fake = session(ANSWERED)
      .turn(turn)
      .turn({ queued: { id: 'event-6' } });
    const { root, fixture } = await open(fake);
    await type(root, 'Set the port to 3000');
    turn.send({
      jsonrpc: '2.0',
      method: 'thread/event',
      params: {
        thread: 'thread-1',
        event: event(4, 'turn.started', {}),
      },
    });
    await fixture.whenStable();
    // The composer stays usable; Stop stays where it was.
    const input = root.querySelector<HTMLTextAreaElement>('[data-testid="thread-composer-input"]');
    expect(input?.disabled).toBe(false);
    expect(root.querySelector('[data-testid="thread-composer-stop"]')).not.toBeNull();
    await type(root, 'Actually, use 4000');
    await fixture.whenStable();
    const waiting = root.querySelector('[data-testid="thread-queued"]');
    expect(waiting?.textContent).toContain('Actually, use 4000');
    expect(waiting?.textContent).toContain('Waiting to be read');
    expect(root.querySelector('[data-testid="thread-composer-stop"]')).not.toBeNull();
    await expectAccessible(root);
    turn.send(
      {
        jsonrpc: '2.0',
        method: 'thread/event',
        params: {
          thread: 'thread-1',
          event: event(5, 'message.added', {
            role: 'user',
            parts: [{ type: 'text', text: 'Set the port to 3000' }],
          }),
        },
      },
      {
        jsonrpc: '2.0',
        method: 'thread/event',
        params: {
          thread: 'thread-1',
          event: event(6, 'message.added', {
            role: 'user',
            parts: [{ type: 'text', text: 'Actually, use 4000' }],
          }),
        },
      },
    );
    await settle();
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thread-queued"]')).toBeNull();
    expect(root.querySelector('[data-testid="thread-message-read"]')?.textContent).toBe(
      'Read while working',
    );
    turn.send({
      jsonrpc: '2.0',
      method: 'thread/event',
      params: { thread: 'thread-1', event: event(7, 'turn.ended', { outcome: 'completed' }) },
    });
    turn.end();
    await settle();
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thread-composer-stop"]')).toBeNull();
  });

  it('keeps glave working under the person’s message as the server records the turn a piece at a time', async () => {
    const turn = new FakeTurn();
    const announce = vi.fn<(said: string, politeness: string) => Promise<void>>(() =>
      Promise.resolve(),
    );
    TestBed.configureTestingModule({
      providers: [{ provide: LiveAnnouncer, useValue: { announce } }],
    });
    const { root, fixture } = await open(session(ANSWERED).turn(turn));
    await type(root, 'Tidy up');
    await fixture.whenStable();
    const working = root.querySelector('[data-testid="thread-working"]');
    expect(working).not.toBeNull();
    // A choice recorded first, then the start, then the message, each on its own.
    for (const each of [
      event(4, 'model.chosen', { model: 'hope-12b' }),
      event(5, 'turn.started', {}),
      event(6, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Tidy up' }] }),
    ]) {
      turn.send({
        jsonrpc: '2.0',
        method: 'thread/event',
        params: { thread: 'thread-1', event: each },
      });
      await settle();
      await fixture.whenStable();
      expect(root.querySelector('[data-testid="thread-working"]')).toBe(working);
    }
    expect(announce.mock.calls.filter(([said]) => said === 'Thinking…')).toHaveLength(1);
    turn.send({
      jsonrpc: '2.0',
      method: 'thread/event',
      params: { thread: 'thread-1', event: event(7, 'turn.ended', { outcome: 'completed' }) },
    });
    turn.end();
    await settle();
    await fixture.whenStable();
  });

  it('sends the model picked with the next turn, and only when it changes', async () => {
    const fake = session([event(1, 'model.chosen', { model: 'gpt-6-luna' })]);
    const { root } = await open(fake);
    root.querySelector<HTMLButtonElement>('[data-testid="model-picker-trigger"]')?.click();
    await settle();
    document.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[1]?.click();
    await settle();
    await type(root, 'Go');
    root.querySelector<HTMLButtonElement>('[data-testid="model-picker-trigger"]')?.click();
    await settle();
    document.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[0]?.click();
    await settle();
    await type(root, 'Again');
    expect(fake.paramsOf('turn/start')).toMatchObject([
      { thread: 'thread-1', prompt: 'Go', model: 'hope-12b' },
      { thread: 'thread-1', prompt: 'Again' },
    ]);
  });

  it('sends the mode picked with the next turn, and only when it changes', async () => {
    // The thread's own mode is Auto (its summary says so), offered first.
    const fake = session([]);
    const { root } = await open(fake);
    // A CDK menu trigger closes what it opens in the task it is made in: the page is drawn late.
    const mode = async (index: number): Promise<void> => {
      await settle();
      root.querySelector<HTMLButtonElement>('[data-testid="permissions-trigger"]')?.click();
      await settle();
      const options = document.querySelectorAll<HTMLElement>('[data-testid="permissions-option"]');
      options.item(index).click();
      await settle();
    };
    expect(root.querySelector('[data-testid="permissions-trigger"]')?.textContent).toContain(
      'Auto',
    );
    // Ask first, after Auto and Plan.
    await mode(2);
    await type(root, 'Careful now');
    await mode(0);
    await type(root, 'Go ahead');
    expect(fake.paramsOf('turn/start')).toMatchObject([
      { thread: 'thread-1', prompt: 'Careful now', permissions: 'ask' },
      { thread: 'thread-1', prompt: 'Go ahead' },
    ]);
  });

  it('draws the conversation from its history while what it changed is never answered', async () => {
    const fake = session(EDITED);
    const changes = fake.held('thread/changes');
    const { root, fixture } = await open(fake);
    expect(root.querySelector('[data-testid="thread-composer"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="thread-message"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="turn-changes"]')).toBeNull();
    // Once read, what it changed comes into its own place.
    changes(README);
    await settle();
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="turn-changes"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="panel-changes"]')).not.toBeNull();
  });

  it('shows the thread’s own model once its history is read, never the default before', async () => {
    const fake = session([event(1, 'model.chosen', { model: 'gpt-6-luna' })]);
    const history = fake.held('thread/events');
    const { root, fixture } = await open(fake);
    const pickers = (): HTMLElement | null =>
      root
        .querySelector<HTMLElement>('[data-testid="model-picker-trigger"]')
        ?.closest('.pending') ?? null;
    // Kept in its place, but unseen while the thread's own choice is unknown.
    expect(pickers()).not.toBeNull();
    history({ events: [event(1, 'model.chosen', { model: 'gpt-6-luna' })] });
    await settle();
    await fixture.whenStable();
    expect(pickers()).toBeNull();
    expect(root.querySelector('[data-testid="model-picker-trigger"]')?.textContent).toContain(
      'GPT-6 Luna',
    );
  });

  it('shows no panel for a plain chat: no edge, no button', async () => {
    const { root } = await open(session(ANSWERED));
    expect(root.querySelector<HTMLElement>('[data-testid="panel-panel"]')?.inert).toBe(true);
    expect(root.querySelector('[data-testid="panel-handle"]')).toBeNull();
    expect(root.querySelector('[data-testid="thread-panel"]')).toBeNull();
    expect(root.querySelector('[data-testid="panel-changes"]')).toBeNull();
    await expectAccessible(root);
  });

  it('shows a thread’s panel beside it, hidden and shown by its edge, button and shortcut', async () => {
    const { root, fixture } = await open(session(EDITED, README));
    const panel = root.querySelector<HTMLElement>('[data-testid="panel-panel"]');
    const edge = root.querySelector<HTMLElement>('[data-testid="panel-handle"]');
    const button = root.querySelector<HTMLElement>('[data-testid="thread-panel"]');
    expect(panel?.inert).toBe(false);
    expect(panel?.querySelector('[data-testid="panel-changes-file"]')?.textContent).toContain(
      'README.md',
    );
    expect(button?.getAttribute('aria-pressed')).toBe('true');
    expect(edge?.getAttribute('aria-keyshortcuts')).toMatch(/^(Meta|Control)\+\.$/u);
    await expectAccessible(root);
    edge?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await fixture.whenStable();
    expect(TestBed.inject(PanelService).showing()).toBe(false);
    expect(panel?.inert).toBe(true);
    expect(button?.getAttribute('aria-pressed')).toBe('false');
    button?.click();
    await fixture.whenStable();
    expect(panel?.inert).toBe(false);
    const apple = /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent);
    document.dispatchEvent(
      new KeyboardEvent('keydown', { key: '.', metaKey: apple, ctrlKey: !apple, bubbles: true }),
    );
    await fixture.whenStable();
    expect(panel?.inert).toBe(true);
  });

  it('opens a file’s changes in full from the panel and from the turn’s card', async () => {
    const { root, fixture } = await open(session(EDITED, README));
    const dialog = (): HTMLDialogElement | null =>
      document.querySelector<HTMLDialogElement>('dialog[data-testid="changes"]');
    expect(dialog()?.open).toBe(false);
    root.querySelector<HTMLButtonElement>('[data-testid="panel-changes-file"]')?.click();
    await fixture.whenStable();
    expect(dialog()?.open).toBe(true);
    const files = dialog()?.querySelectorAll('[data-testid="file"]');
    expect(files).toHaveLength(1);
    expect(files?.[0]?.textContent).toContain('-teh');
    await expectAccessible(root);
    dialog()?.querySelector<HTMLButtonElement>('[data-testid="changes-close"]')?.click();
    await fixture.whenStable();
    expect(dialog()?.open).toBe(false);
    root.querySelector<HTMLButtonElement>('[data-testid="turn-changes-file"]')?.click();
    await fixture.whenStable();
    expect(dialog()?.open).toBe(true);
    fixture.componentRef.setInput('thread', 'thread-2');
    await fixture.whenStable();
    expect(dialog()?.open).toBe(false);
  });

  it('says in the composer’s place, in the server’s words, that the project’s folder is gone', async () => {
    const gone = "This project's folder isn't there anymore.";
    const fake = session(ANSWERED).answer('project/list', {
      projects: [record('p1', 'glave')],
      gone: [{ project: 'p1', message: gone }],
    });
    const { root } = await open(fake);
    expect(root.querySelector('[data-testid="thread-gone-message"]')?.textContent.trim()).toBe(
      gone,
    );
    expect(root.querySelector('[data-testid="thread-gone-find"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="thread-gone-remove"]')).not.toBeNull();
    expect(
      root.querySelector('[data-testid="thread-composer"]')?.closest('[hidden]'),
    ).not.toBeNull();
    expect(root.querySelector('[data-testid="thread-error"]')).toBeNull();
    await expectAccessible(root);
  });

  it('shows a proposed plan as a card: Change it goes to the composer, Edit saves, Build it builds', async () => {
    const plan = {
      steps: [
        { step: 'Read how flags are parsed', status: 'pending' },
        { step: 'Add --verbose', status: 'pending' },
      ],
      proposed: true,
      sentence: 'Proposed plan: 2 steps',
    } as const;
    const edited = { ...plan, steps: [plan.steps[1]], sentence: 'Proposed plan: 1 step' };
    const proposed = [...ANSWERED, event(4, 'plan.updated', { steps: [], proposed: true })];
    const fake = session(proposed)
      .answer('thread/plan', { plan })
      .answer('plan/edit', { plan: edited });
    const { root, fixture } = await open(fake);
    const card = root.querySelector('[data-testid="thread-plan"]');
    expect(card?.textContent).toContain('Proposed plan: 2 steps');
    await expectAccessible(root);
    const click = async (testId: string): Promise<void> => {
      root.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`)?.click();
      await settle();
      await fixture.whenStable();
    };
    await click('thread-plan-change');
    expect(document.activeElement?.getAttribute('data-testid')).toBe('thread-composer-input');
    await click('thread-plan-edit');
    await click('thread-plan-remove');
    await click('thread-plan-save');
    expect(fake.paramsOf('plan/edit')).toEqual([{ thread: 'thread-1', steps: ['Add --verbose'] }]);
    expect(card?.textContent).toContain('Proposed plan: 1 step');
    await click('thread-plan-build');
    expect(fake.paramsOf('plan/build')).toEqual([{ thread: 'thread-1' }]);
  });

  it('renames the thread through the dialog', async () => {
    const fake = session([]);
    const { root, fixture } = await open(fake);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-rename"]')?.click();
    await fixture.whenStable();
    const input = root.querySelector<HTMLInputElement>('[data-testid="rename-input"]');
    expect(input?.value).toBe('Tidy the repo');
    if (input) {
      input.value = 'Spring cleaning';
      input.dispatchEvent(new Event('input'));
    }
    await fixture.whenStable();
    root.querySelector<HTMLButtonElement>('[data-testid="rename-save"]')?.click();
    await settle();
    expect(fake.paramsOf('thread/rename')).toEqual([
      { thread: 'thread-1', title: 'Spring cleaning' },
    ]);
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thread-title"]')?.textContent.trim()).toBe(
      'Spring cleaning',
    );
  });

  it('shows a thread older than the side nav lists by its own title, mode and project', async () => {
    const older = thread('thread-1', 'Old release notes', {
      project: 'p1',
      permissions: 'ask',
      updated_at: '2026-09-01T10:00:00.000Z',
    });
    const fake = session([])
      .answer('thread/list', { threads: [] })
      .answer('thread/get', { thread: older })
      .answer('file/upload', {
        file: { sha256: 'a'.repeat(64), name: 'notes.txt', media_type: 'text/plain', bytes: 5 },
      });
    const { root, fixture } = await open(fake);
    expect(root.querySelector('[data-testid="thread-title"]')?.textContent.trim()).toBe(
      'Old release notes',
    );
    expect(root.querySelector('[data-testid="thread-meta"]')?.textContent.trim()).toBe('glave');
    expect(root.querySelector('[data-testid="permissions-trigger"]')?.textContent).toContain('Ask');
    await expectAccessible(root);
    // A file added to a message in it is kept in its project.
    const picker = root.querySelector<HTMLInputElement>('[data-testid="thread-composer-picker"]');
    const file = new File(['notes'], 'notes.txt', { type: 'text/plain' });
    Object.defineProperty(picker, 'files', { configurable: true, value: [file] });
    picker?.dispatchEvent(new Event('change'));
    await vi.waitFor(() => {
      expect(fake.paramsOf('file/upload')).toMatchObject([{ name: 'notes.txt', project: 'p1' }]);
    });
    await fixture.whenStable();
  });
});

describe('ThreadComponent with files', () => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(URL, 'createObjectURL', {
      configurable: true,
      value: () => 'blob:test/1',
    });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: () => undefined });
  });

  it('adds a file to the message, keeps it in the project, and sends it with the message', async () => {
    const sha = 'a'.repeat(64);
    const fake = session([])
      .answer('file/upload', {
        file: { sha256: sha, name: 'cat.png', media_type: 'image/png', bytes: 3 },
      })
      .streams([
        {
          jsonrpc: '2.0',
          method: 'thread/event',
          params: {
            thread: 'thread-1',
            event: event(1, 'message.added', {
              role: 'user',
              parts: [
                { type: 'text', text: 'What is this?' },
                {
                  type: 'file',
                  name: 'cat.png',
                  blob: { sha256: sha, bytes: 3, media_type: 'image/png' },
                },
              ],
            }),
          },
        },
      ]);
    const { root, fixture } = await open(fake);
    const picker = root.querySelector<HTMLInputElement>('[data-testid="thread-composer-picker"]');
    expect(picker).not.toBeNull();
    const file = new File(['png'], 'cat.png', { type: 'image/png' });
    Object.defineProperty(picker, 'files', { configurable: true, value: [file] });
    picker?.dispatchEvent(new Event('change'));
    await vi.waitFor(() => {
      expect(fake.paramsOf('file/upload')).toHaveLength(1);
    });
    await fixture.whenStable();
    const chip = root.querySelector('[data-testid="thread-composer-file"]');
    expect(chip?.textContent).toContain('cat.png');
    expect(chip?.querySelector('img')?.getAttribute('src')).toBe('blob:test/1');
    await expectAccessible(root);
    await type(root, 'What is this?');
    expect(fake.paramsOf('turn/start')).toMatchObject([
      { thread: 'thread-1', prompt: 'What is this?', files: [{ sha256: sha, name: 'cat.png' }] },
    ]);
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thread-composer-file"]')).toBeNull();
    const sent = root.querySelector('[data-testid="thread-message-file"]');
    expect(sent?.textContent).toContain('cat.png');
    expect(sent?.textContent).toContain('3 bytes');
  });

  it('reads the thread again in the person’s language once they choose another', async () => {
    const fake = session(HISTORY);
    const { root, fixture } = await open(fake);
    expect(fake.paramsOf('thread/events')).toHaveLength(1);
    // What the approval asks, as the server writes it in the language chosen now.
    const asked = { ...HISTORY[2], data: { ...HISTORY[2]?.data, ask: '[Ŕûñ ţĥîš? one]' } };
    fake.answer('thread/events', { events: [HISTORY[0], HISTORY[1], asked, HISTORY[3]] });
    TestBed.inject(ChangesService).tell('language');
    await fixture.whenStable();
    await settle();
    await fixture.whenStable();
    expect(fake.paramsOf('thread/events')).toHaveLength(2);
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.querySelector('.summary')?.textContent).toContain('[Ŕûñ ţĥîš? one]');
  });
});
