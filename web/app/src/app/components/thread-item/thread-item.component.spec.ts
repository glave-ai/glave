import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import type { ThreadItem } from '../../lib/conversation';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ThreadItemComponent } from './thread-item.component';

async function render(
  item: ThreadItem,
  fake = new FakeSession(),
): Promise<{ readonly root: HTMLElement; readonly fake: FakeSession }> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadItemComponent);
  fixture.componentRef.setInput('item', item);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fake };
}

describe('ThreadItemComponent', () => {
  it('shows a reply as markdown, named for screen readers, with a copy button', async () => {
    const { root } = await render({
      kind: 'message',
      key: 'm',
      seq: 1,
      from: 'model',
      text: 'It **works**',
      reasoning: 'Checked it',
    });
    const message = root.querySelector('[data-testid="thread-message"]');
    expect(message?.getAttribute('aria-label')).toBe('Reply');
    expect(message?.querySelector('strong')?.textContent).toBe('works');
    expect(message?.querySelector('glz-reasoning')).not.toBeNull();
    expect(message?.querySelector('[data-testid="message-copy"]')).not.toBeNull();
    await expectAccessible(root);
  });

  it('gives a message with nothing to copy no copy button', async () => {
    const { root } = await render({
      kind: 'message',
      key: 'm',
      seq: 1,
      from: 'model',
      text: '',
      reasoning: 'Thinking it over',
    });
    expect(root.querySelector('[data-testid="message-copy"]')).toBeNull();
  });

  it('shows a call as a quiet line with its tool’s icon', async () => {
    const { root } = await render({
      kind: 'call',
      key: 'c',
      seq: 1,
      tool: 'bash',
      summary: 'cargo test',
      status: 'failed',
      output: 'FAILED',
    });
    const receipt = root.querySelector('[data-testid="thread-receipt"]');
    expect(receipt?.querySelector('.tool')?.textContent).toBe('bash');
    expect(receipt?.querySelector('.kind svg')).not.toBeNull();
    expect(receipt?.querySelector('pre')?.textContent).toBe('FAILED');
    await expectAccessible(root);
  });

  it('asks for an approval in a sentence, with what would run, and sends the answer', async () => {
    const { root, fake } = await render({
      kind: 'approval',
      key: 'a',
      seq: 1,
      approval: 'c1',
      tool: 'bash',
      summary: 'bash: git push -u origin fix/worktree-race',
      decision: null,
    });
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.querySelector('.label')?.textContent).toBe('Needs you');
    expect(card?.querySelector('.summary')?.textContent).toContain('Run this command?');
    expect(card?.querySelector('.detail')?.textContent).toBe(
      'git push -u origin fix/worktree-race',
    );
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-approval-allow-thread"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c1', decision: 'allow_for_thread' },
    ]);
  });

  it('asks questions and sends the answers', async () => {
    const { root, fake } = await render({
      kind: 'question',
      key: 'q',
      seq: 1,
      call: 'c2',
      questions: [{ question: 'Keep logs?', options: [{ label: 'Yes' }] }],
      answers: null,
    });
    root.querySelector<HTMLInputElement>('[data-testid="thread-question-choice"] input')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    root.querySelector<HTMLButtonElement>('[data-testid="thread-question-submit"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('question/answer')).toEqual([
      { thread: 'thread-1', question: 'c2', answers: ['Yes'] },
    ]);
  });
});
