import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { SpacesService } from '../../services/spaces.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MODELS, SERVER, event, space, thread } from '../../testing/fixtures';
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
    .answer('tasks/list', { tasks: [] })
    .answer('space/list', { spaces: [space('p1', 'glave')] })
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
  await TestBed.inject(SpacesService).refresh();
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
  await new Promise((resolve) => setTimeout(resolve, 10));
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
    expect(root.querySelector('[data-testid="thread-title"]')?.textContent).toBe('Tidy the repo');
    expect(root.querySelector('[data-testid="thread-space"]')?.textContent).toBe('glave');
    expect(root.querySelector('[data-testid="thread-message"]')?.textContent).toContain('Clean up');
    expect(root.querySelector('[data-testid="thread-receipt"]')?.textContent).toContain(
      'rm -rf build',
    );
    expect(root.querySelector('[data-testid="thread-question"]')?.textContent).toContain(
      'Keep logs?',
    );
    expect(root.querySelector('[data-testid="approval-mode"]')?.textContent).toContain('Auto');
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-approval-allow-once"]')?.click();
    await settle();
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c1', decision: 'allow_once' },
    ]);
  });

  it('names no model over a reply, and gives every message a copy button', async () => {
    const { root } = await open(session(ANSWERED));
    const messages = root.querySelectorAll('[data-testid="thread-message"]');
    expect(messages[0]?.getAttribute('aria-label')).toBe('You');
    expect(messages[1]?.getAttribute('aria-label')).toBe('Reply');
    expect(root.querySelector('glz-avatar')).toBeNull();
    expect(root.querySelector('[data-testid="thread-items"] .author')).toBeNull();
    expect(root.querySelector('[data-testid="thread-items"]')?.textContent).not.toContain('Hope');
    expect(messages[0]?.querySelector('app-message-text')?.textContent).toContain('Hi');
    for (const message of messages) {
      expect(message.querySelector('.actions [data-testid="message-copy"]')).not.toBeNull();
    }
    await expectAccessible(root);
  });

  it('folds tool calls in a row under one line that opens to each, its tool and target', async () => {
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
    const steps = root.querySelector('[data-testid="thread-steps"]');
    const summary = root.querySelector<HTMLElement>('[data-testid="thread-steps-summary"]');
    expect(summary?.textContent.trim()).toBe('Ran 3 steps');
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
    expect(fake.paramsOf('turn/start')).toEqual([{ thread: 'thread-1', prompt: 'Go' }]);
    expect(root.textContent).toContain('Done.');
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
    expect(fake.paramsOf('turn/start')).toEqual([
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
    expect(fake.paramsOf('turn/start')).toEqual([
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
