import { LiveAnnouncer } from '@angular/cdk/a11y';
import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { ProjectsService } from '../../services/projects.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { FakeSession, FakeTurn, provideFakeSession } from '../../testing/fake-session';
import { MODELS, SERVER, event, record, thread } from '../../testing/fixtures';
import { ThreadComponent } from './thread.component';

const HISTORY = [
  event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Clean up' }] }),
  event(2, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'bash',
    arguments: { json: { command: 'rm -rf build' } },
  }),
  event(3, 'approval.requested', { call_id: 'c1', tool: 'bash', summary: 'Delete build/' }),
  event(4, 'tool.call.requested', {
    call_id: 'c2',
    tool: 'question',
    arguments: { json: { questions: [{ question: 'Keep logs?', options: [{ label: 'Yes' }] }] } },
  }),
];

const ANSWERED = [
  event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Hi' }] }),
  event(2, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Hello' }] }),
  event(3, 'model.responded', { model: { provider: 'gateway', name: 'hope-12b' } }),
];

function session(events: readonly unknown[]): FakeSession {
  return new FakeSession()
    .answer('thread/events', { events })
    .answer('model/list', { models: MODELS })
    .answer('server/discover', SERVER)
    .answer('schedule/list', { schedules: [] })
    .answer('tasks/list', { tasks: [] })
    .answer('project/list', { projects: [record('p1', 'glave')] })
    .answer('thread/list', { threads: [thread('thread-1', 'Tidy the repo')] });
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

async function open(
  fake: FakeSession,
  narrow = false,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadComponent>>;
}> {
  const breakpoints = { observe: () => of({ matches: narrow, breakpoints: {} }) };
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      { provide: BreakpointObserver, useValue: breakpoints },
    ],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(ThreadComponent);
  fixture.componentRef.setInput('thread', 'thread-1');
  await fixture.whenStable();
  await settle();
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

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
    // The approval mode and the model in the composer's quiet line.
    const status = root.querySelector('[data-testid="thread-composer-status"]');
    expect(status?.querySelector('[data-testid="approval-mode"]')?.textContent).toContain('Auto');
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
        { text: 'Lets glave approve actions without asking' },
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
      'Lets glave approve actions without asking',
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

  it('shows a turn that worked results first: the reply, what changed, then its steps', async () => {
    const call = (seq: number, id: string, tool: string, json: Record<string, unknown>): unknown =>
      event(seq, 'tool.call.requested', { call_id: id, tool, arguments: { json } });
    const done = (seq: number, id: string): unknown =>
      event(seq, 'tool.call.completed', { call_id: id, is_error: false, output: [] });
    const { root, fixture } = await open(
      session([
        event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Fix it' }] }),
        call(2, 'a', 'bash', { command: 'cargo test' }),
        done(3, 'a'),
        call(4, 'b', 'edit', { path: 'src/parse.rs' }),
        done(5, 'b'),
        call(6, 'c', 'write', { path: 'tests/empty.rs' }),
        done(7, 'c'),
        event(8, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Done' }] }),
      ]),
    );
    const order = Array.from(
      root.querySelectorAll(
        '[data-testid="thread-message"], [data-testid="turn-changes"], [data-testid="thread-steps"]',
      ),
    ).map((each) => each.getAttribute('data-testid'));
    expect(order).toEqual(['thread-message', 'thread-message', 'turn-changes', 'thread-steps']);
    expect(root.querySelector('[data-testid="turn-changes-count"]')?.textContent).toBe('2 files');
    // Changes in the bar too, for the whole thread.
    expect(root.querySelector('[data-testid="thread-changes"]')?.textContent).toContain('2');
    const steps = root.querySelector('[data-testid="thread-steps"]');
    const summary = root.querySelector<HTMLElement>('[data-testid="thread-steps-summary"]');
    expect(summary?.querySelector('.state')?.textContent).toBe('Done');
    expect(summary?.querySelector('.count')?.textContent).toBe('3 steps');
    expect(steps?.querySelector('details')?.open).toBe(false);
    const receipts = Array.from(steps?.querySelectorAll('[data-testid="thread-receipt"]') ?? []);
    expect(receipts.map((receipt) => receipt.querySelector('.tool')?.textContent)).toEqual([
      'bash',
      'edit',
      'write',
    ]);
    expect(receipts.map((receipt) => receipt.querySelector('.what')?.textContent)).toEqual([
      'cargo test',
      'src/parse.rs',
      'tests/empty.rs',
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

  it('never sends as it opens: it shows what the server has', async () => {
    const fake = session(ANSWERED).streams([]);
    const { root } = await open(fake);
    await settle();
    expect(fake.paramsOf('turn/start')).toEqual([]);
    expect(root.querySelector('[data-testid="thread-message"]')?.textContent).toContain('Hi');
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

  it('sends the approval mode picked with the next turn, and only when it changes', async () => {
    // The thread's own mode is auto (its summary says so).
    const fake = session([]);
    const { root } = await open(fake);
    const mode = async (index: number): Promise<void> => {
      root.querySelector<HTMLButtonElement>('[data-testid="approval-mode-trigger"]')?.click();
      await settle();
      const options = document.querySelectorAll<HTMLElement>(
        '[data-testid="approval-mode-option"]',
      );
      options.item(index).click();
      await settle();
    };
    expect(root.querySelector('[data-testid="approval-mode-trigger"]')?.textContent).toContain(
      'Auto',
    );
    await mode(0);
    await type(root, 'Careful now');
    await mode(1);
    await type(root, 'Go ahead');
    expect(fake.paramsOf('turn/start')).toMatchObject([
      { thread: 'thread-1', prompt: 'Careful now', approval: 'ask' },
      { thread: 'thread-1', prompt: 'Go ahead' },
    ]);
  });

  it('opens and closes the tools panel by its edge and its shortcut on wide screens', async () => {
    const { root, fixture } = await open(session([]));
    const panel = root.querySelector<HTMLElement>('[data-testid="tools-panel"]');
    const edge = root.querySelector<HTMLElement>('[data-testid="tools-handle"]');
    expect(root.querySelector('[data-testid="thread-tools"]')).toBeNull();
    expect(root.querySelector('[data-testid="tools-close"]')).toBeNull();
    expect(panel?.inert).toBe(true);
    expect(edge?.getAttribute('aria-keyshortcuts')).toMatch(/^(Meta|Control)\+\.$/u);
    edge?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await fixture.whenStable();
    expect(TestBed.inject(ToolsPanelService).open()).toBe(true);
    expect(panel?.inert).toBe(false);
    expect(root.querySelector('app-tools-panel [data-testid="tools-tabs"]')).not.toBeNull();
    await expectAccessible(root);
    const apple = /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent);
    document.dispatchEvent(
      new KeyboardEvent('keydown', { key: '.', metaKey: apple, ctrlKey: !apple, bubbles: true }),
    );
    await fixture.whenStable();
    expect(panel?.inert).toBe(true);
  });

  it('opens and closes the tools sheet from the top bar on narrow screens', async () => {
    const { root, fixture } = await open(session([]), true);
    const toggle = root.querySelector<HTMLButtonElement>('[data-testid="thread-tools"]');
    const panel = root.querySelector<HTMLElement>('[data-testid="tools-panel"]');
    expect(root.querySelector('[data-testid="tools-handle"]')).toBeNull();
    expect(toggle?.getAttribute('aria-pressed')).toBe('false');
    toggle?.click();
    await fixture.whenStable();
    expect(toggle?.getAttribute('aria-pressed')).toBe('true');
    expect(panel?.getAttribute('aria-modal')).toBe('true');
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="tools-close"]')?.click();
    await fixture.whenStable();
    expect(panel?.inert).toBe(true);
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
});
