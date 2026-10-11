import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { event } from '../../testing/fixtures';
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

async function open(fake: FakeSession): Promise<HTMLElement> {
  TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
  const fixture = TestBed.createComponent(ThreadComponent);
  fixture.componentRef.setInput('thread', 'thread-1');
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('ThreadComponent', () => {
  it('shows the conversation with its receipts, approvals and questions, answered in place', async () => {
    const fake = new FakeSession().answer('thread/events', { events: HISTORY });
    const root = await open(fake);
    expect(root.querySelector('[data-testid="thread-message"]')?.textContent).toContain('Clean up');
    expect(root.querySelector('[data-testid="thread-receipt"]')?.textContent).toContain(
      'rm -rf build',
    );
    expect(root.querySelector('[data-testid="thread-question"]')?.textContent).toContain(
      'Keep logs?',
    );
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="approval-allow-once"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c1', decision: 'allow_once' },
    ]);
  });

  it('sends from the composer and shows the reply streaming in', async () => {
    const fake = new FakeSession().answer('thread/events', { events: [] }).streams([
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
    const root = await open(fake);
    const input = root.querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]');
    if (input) {
      input.value = 'Go';
      input.dispatchEvent(new Event('input'));
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(fake.paramsOf('turn/start')).toEqual([{ thread: 'thread-1', prompt: 'Go' }]);
    expect(root.textContent).toContain('Done.');
  });
});
