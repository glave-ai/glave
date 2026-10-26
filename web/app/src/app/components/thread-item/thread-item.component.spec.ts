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
      files: [],
      key: 'm',
      seq: 1,
      from: 'model',
      text: 'It **works**',
      reasoning: 'Checked it',
      steered: false,
    });
    const message = root.querySelector('[data-testid="thread-message"]');
    expect(message?.getAttribute('aria-label')).toBe('Reply');
    expect(message?.querySelector('strong')?.textContent).toBe('works');
    expect(message?.querySelector('glz-reasoning')).not.toBeNull();
    expect(message?.querySelector('[data-testid="message-copy"]')).not.toBeNull();
    await expectAccessible(root);
  });

  it('gives the person’s own message no copy button', async () => {
    const { root } = await render({
      kind: 'message',
      files: [],
      key: 'm',
      seq: 1,
      from: 'person',
      text: 'Fix the flaky test',
      reasoning: '',
      steered: false,
    });
    expect(root.querySelector('[data-testid="message-copy"]')).toBeNull();
  });

  it('shows the approval of a plan as the person’s Build it', async () => {
    const { root } = await render({
      kind: 'message',
      files: [],
      key: 'm',
      seq: 1,
      from: 'person',
      text: '',
      reasoning: '',
      steered: false,
      built: true,
    });
    const message = root.querySelector('[data-testid="thread-message"]');
    expect(message?.getAttribute('aria-label')).toBe('You');
    expect(message?.textContent.trim()).toBe('Build it');
    await expectAccessible(root);
  });

  it('says the model read a message the person sent while it worked', async () => {
    const message = {
      kind: 'message',
      files: [],
      key: 'm',
      seq: 4,
      from: 'person',
      text: 'Actually, use 4000',
      reasoning: '',
    } as const;
    const read = await render({ ...message, steered: true });
    const caption = read.root.querySelector('[data-testid="thread-message-read"]');
    expect(caption?.textContent).toBe('Read while working');
    await expectAccessible(read.root);
    TestBed.resetTestingModule();
    const asked = await render({ ...message, steered: false });
    expect(asked.root.querySelector('[data-testid="thread-message-read"]')).toBeNull();
  });

  it('gives a message with nothing to copy no copy button', async () => {
    const { root } = await render({
      kind: 'message',
      files: [],
      key: 'm',
      seq: 1,
      from: 'model',
      text: '',
      reasoning: 'Thinking it over',
      steered: false,
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
      summary: 'bash: cargo test --workspace',
      caution: '',
      outsideSandbox: false,
      withNetwork: false,
      modelReason: '',
      decision: null,
    });
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.querySelector('.label')?.textContent).toBe('Needs you');
    expect(card?.querySelector('.summary')?.textContent).toContain('Run this command?');
    expect(card?.querySelector('.detail')?.textContent).toBe('cargo test --workspace');
    // Only the mode asks, so there is no reason to give.
    expect(root.querySelector('[data-testid="thread-approval-caution"]')).toBeNull();
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-approval-allow-thread"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c1', decision: 'allow_for_thread' },
    ]);
  });

  it('says why a risky command asks in a sentence of its own, never inside the command', async () => {
    const { root } = await render({
      kind: 'approval',
      key: 'a',
      seq: 1,
      approval: 'c1',
      tool: 'bash',
      summary: 'bash: git push -u origin fix/worktree-race',
      caution: 'It pushes to a remote.',
      outsideSandbox: false,
      withNetwork: false,
      modelReason: '',
      decision: null,
    });
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.querySelector('.detail')?.textContent).toBe(
      'git push -u origin fix/worktree-race',
    );
    const caution = root.querySelector('[data-testid="thread-approval-caution"]');
    expect(caution?.textContent).toBe('It pushes to a remote.');
    expect(card?.querySelector('.summary')?.contains(caution)).toBe(false);
    await expectAccessible(root);
  });

  it('asks to run a command outside the sandbox once, with the model’s reason as its words', async () => {
    const { root, fake } = await render({
      kind: 'approval',
      key: 'a',
      seq: 1,
      approval: 'c1',
      tool: 'bash',
      summary: 'bash: cargo add serde',
      caution: 'It runs outside the sandbox, where it can read and change any file you can.',
      outsideSandbox: true,
      withNetwork: false,
      modelReason: 'cargo add writes to ~/.cargo/registry, which the sandbox blocks.',
      decision: null,
    });
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.querySelector('.summary')?.textContent).toContain('Run this command?');
    expect(card?.querySelector('.detail')?.textContent).toBe('cargo add serde');
    expect(root.querySelector('[data-testid="thread-approval-caution"]')?.textContent).toBe(
      'It runs outside the sandbox, where it can read and change any file you can.',
    );
    const reason = root.querySelector('[data-testid="thread-approval-reason"]');
    expect(reason?.textContent.replace(/\s+/gu, ' ').trim()).toBe(
      'The model says cargo add writes to ~/.cargo/registry, which the sandbox blocks.',
    );
    expect(reason?.querySelector('q')).not.toBeNull();
    // Each one asks: no answer for the rest of the thread.
    expect(root.querySelector('[data-testid="thread-approval-allow-thread"]')).toBeNull();
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-approval-allow-once"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c1', decision: 'allow_once' },
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
