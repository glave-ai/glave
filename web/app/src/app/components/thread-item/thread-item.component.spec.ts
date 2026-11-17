import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { GlzDecision } from '@glave/glaze';
import { expectAccessible } from '@glave/glaze/testing';

import {
  type ApprovalItem,
  NO_THOUGHT,
  type RepeatAsk,
  type ServiceItem,
  type ThreadItem,
} from '../../lib/conversation';
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

/** The lines of a card asking to repeat work, as the server writes them. */
const LINES = [
  'Monday to Friday, 8:00 am your time. First run: Tuesday 17 November.',
  'Each run is added to its own chat, “Morning check”.',
  'Each run starts with: “Check the build.”',
];

/** A card asking to repeat work, answered `decision`, with what its call came to. */
function repeating(
  decision: GlzDecision | null,
  done: Pick<RepeatAsk, 'made' | 'refused'> = { made: null, refused: null },
): ApprovalItem {
  return {
    kind: 'approval',
    key: 'a',
    seq: 1,
    approval: 'c2',
    tool: 'schedule',
    summary: 'schedule: First run',
    ask: 'Repeat this every weekday at 8:00 am?',
    detail: LINES.join('\n'),
    caution: 'It runs only while glave runs on this computer.',
    outsideSandbox: false,
    withNetwork: false,
    modelReason: '',
    repeat: { lines: LINES, ...done },
    decision,
  };
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
      thought: { headline: 'Checked it', ms: 4250 },
      steered: false,
    });
    const message = root.querySelector('[data-testid="thread-message"]');
    expect(message?.getAttribute('aria-label')).toBe('Reply');
    expect(message?.querySelector('strong')?.textContent).toBe('works');
    // Its reasoning in one line, as the server says it went.
    expect(message?.querySelector('[data-testid="thread-thought-label"]')?.textContent).toBe(
      'Thought for 4s',
    );
    expect(message?.querySelector('[data-testid="thread-thought-headline"]')?.textContent).toBe(
      'Checked it',
    );
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
      thought: NO_THOUGHT,
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
      thought: NO_THOUGHT,
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
      thought: NO_THOUGHT,
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
      thought: NO_THOUGHT,
      steered: false,
    });
    expect(root.querySelector('[data-testid="message-copy"]')).toBeNull();
  });

  it('shows a call as a quiet line, the server’s, with its tool’s icon', async () => {
    const { root } = await render({
      kind: 'call',
      key: 'c',
      seq: 1,
      call: 'c',
      tool: 'bash',
      summary: 'bash: cargo test',
      status: 'failed',
      output: 'FAILED',
      images: [],
    });
    const receipt = root.querySelector('[data-testid="thread-receipt"]');
    expect(receipt?.querySelector('.what')?.textContent).toBe('bash: cargo test');
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
      ask: 'Run this command?',
      detail: 'cargo test --workspace',
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

  it('asks to repeat work in the server’s words, answered Repeat it or Not now', async () => {
    const { root, fake } = await render(repeating(null));
    const card = root.querySelector('[data-testid="thread-repeat"]');
    expect(card?.querySelector('.summary')?.textContent.trim()).toBe(
      'Repeat this every weekday at 8:00 am?',
    );
    const lines = [...(card?.querySelectorAll('[data-testid="thread-repeat-lines"] li') ?? [])];
    expect(lines.map((line) => line.textContent.trim())).toEqual(LINES);
    // Each line after its icon: when it runs, its chat, what it starts with; the question after
    // the repeat's.
    expect(lines.map((line) => line.querySelectorAll('glz-icon').length)).toEqual([1, 1, 1]);
    expect(card?.querySelector('.summary glz-icon')).not.toBeNull();
    expect(card?.querySelector('code')).toBeNull();
    expect(card?.querySelector('[data-testid="thread-repeat-caution"]')?.textContent).toBe(
      'It runs only while glave runs on this computer.',
    );
    expect(card?.classList).toContain('glz-approval-card--paired');
    const buttons = [...(card?.querySelectorAll('button') ?? [])];
    expect(buttons.map((each) => each.textContent.trim())).toEqual(['Repeat it', 'Not now']);
    await expectAccessible(root);
    card?.querySelector<HTMLButtonElement>('[data-testid="thread-repeat-allow-once"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'c2', decision: 'allow_once' },
    ]);
  });

  it('folds a repeat the call made to the server’s line, which opens its chat', async () => {
    const made = { chat: 'chat-1', repeats: 'Every weekday at 8:00 am' };
    const { root } = await render(repeating('allow_once', { made, refused: null }));
    expect(root.querySelector('[data-testid="thread-repeat"]')).toBeNull();
    const link = root.querySelector('[data-testid="thread-repeat-chat"]');
    expect(link?.textContent).toBe('Every weekday at 8:00 am');
    expect(link?.getAttribute('href')).toBe('/t/chat-1');
    await expectAccessible(root);
  });

  it('says in the server’s words why it made none, and shows nothing meanwhile or once declined', async () => {
    const refused = 'You have as many schedules as you may keep.';
    const failed = await render(repeating('allow_once', { made: null, refused }));
    expect(
      failed.root.querySelector('[data-testid="thread-repeat-refused"]')?.textContent.trim(),
    ).toBe(refused);
    expect(failed.root.querySelector('a')).toBeNull();
    for (const decision of ['allow_once', 'deny'] as const) {
      TestBed.resetTestingModule();
      const { root } = await render(repeating(decision));
      expect(root.textContent.trim()).toBe('');
    }
  });

  it('asks in the server’s sentence and shows what would run as it gives it', async () => {
    const { root } = await render({
      kind: 'approval',
      key: 'a',
      seq: 1,
      approval: 'c1',
      tool: 'mcp__tracker__close',
      summary: 'mcp__tracker__close {"issue":12}',
      ask: 'Close this issue?',
      detail: 'Issue 12',
      caution: '',
      outsideSandbox: false,
      withNetwork: false,
      modelReason: '',
      decision: null,
    });
    const card = root.querySelector('[data-testid="thread-approval"]');
    expect(card?.querySelector('.summary')?.textContent).toContain('Close this issue?');
    expect(card?.querySelector('.detail')?.textContent).toBe('Issue 12');
    await expectAccessible(root);
  });

  it('says why a risky command asks in a sentence of its own, never inside the command', async () => {
    const { root } = await render({
      kind: 'approval',
      key: 'a',
      seq: 1,
      approval: 'c1',
      tool: 'bash',
      summary: 'bash: git push -u origin fix/worktree-race',
      ask: 'Run this command?',
      detail: 'git push -u origin fix/worktree-race',
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
      ask: 'Run this command?',
      detail: 'cargo add serde',
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

  /** What a service asks, as `thread/events` has it. */
  const service = (told: string | null, detail: ServiceItem['detail'] = null): ServiceItem => ({
    kind: 'service',
    key: 'service-7',
    seq: 7,
    call: 'c3',
    service: 'GitHub',
    ask: 'GitHub asks you, not the model',
    note: 'Your answer goes only to GitHub, as the server says.',
    message: 'Which label?',
    fields: [
      {
        name: 'label',
        label: 'Label',
        required: true,
        kind: { type: 'choice', options: [{ value: 'bug', label: 'Bug' }] },
      },
    ],
    detail,
    told,
  });

  it('folds what glave adds under its question, shut until opened', async () => {
    const detail = {
      front: 'name: tidy',
      text: '# Tidy',
      textCut: false,
      files: [{ path: 'scripts/tidy.sh', text: '#!/bin/sh', runsCode: true, cut: false }],
      cut: null,
    };
    const { root } = await render(service(null, detail));
    const fold = root.querySelector<HTMLElement>('[data-testid="thread-service-detail-summary"]');
    expect(fold?.textContent.trim()).toBe('What it adds');
    expect(
      root.querySelector('[data-testid="thread-service-detail"] details')?.hasAttribute('open'),
    ).toBe(false);
    fold?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(root.querySelector('[data-testid="thread-service-detail-runs"]')?.textContent).toBe(
      'runs code',
    );
    await expectAccessible(root);
  });

  it('says who asks and where the answer goes, and sends it to the service', async () => {
    const { root, fake } = await render(service(null));
    const part = (name: string): HTMLElement | null =>
      root.querySelector(`[data-testid="thread-service-${name}"]`);
    expect(part('who')?.textContent).toBe('GitHub asks you, not the model');
    expect(part('note')?.textContent).toBe('Your answer goes only to GitHub, as the server says.');
    expect(part('send')?.textContent.trim()).toBe('Send to GitHub');
    await expectAccessible(root);
    root.querySelector<HTMLInputElement>('[data-testid="thread-service-choice"] input')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    part('send')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('service/answer')).toEqual([
      {
        thread: 'thread-1',
        call: 'c3',
        asked: 7,
        answer: { answered: { values: { label: 'bug' } } },
      },
    ]);
  });

  it('declines', async () => {
    const { root, fake } = await render(service(null));
    root.querySelector<HTMLButtonElement>('[data-testid="thread-service-decline"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('service/answer')).toEqual([
      { thread: 'thread-1', call: 'c3', asked: 7, answer: 'declined' },
    ]);
  });

  it('rests as the server line once it was answered', async () => {
    const told = await render(service('You declined GitHub’s question'));
    expect(told.root.querySelector('[data-testid="thread-service-told"]')?.textContent).toBe(
      'You declined GitHub’s question',
    );
    expect(told.root.querySelector('[data-testid="thread-service-send"]')).toBeNull();
  });
});
