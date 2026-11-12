import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { NO_THOUGHT, type CallItem, type MessageItem } from '../../lib/conversation';
import type { TurnPart } from '../../lib/runs';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { TurnPartComponent } from './turn-part.component';

const CALL: CallItem = {
  kind: 'call',
  key: 'c2',
  seq: 2,
  call: 'c2',
  tool: 'bash',
  summary: 'bash: exit 3',
  status: 'failed',
  output: 'exit 3',
  images: [],
};

const SAID: MessageItem = {
  kind: 'message',
  files: [],
  key: 'm4',
  seq: 4,
  from: 'model',
  text: 'It fails; looking closer.',
  reasoning: '',
  thought: NO_THOUGHT,
  steered: false,
};

async function render(part: TurnPart): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(new FakeSession()), ThreadService],
  });
  const fixture = TestBed.createComponent(TurnPartComponent);
  fixture.componentRef.setInput('part', part);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('TurnPartComponent', () => {
  it('shows a run as the server’s line, closed, which opens to its calls', async () => {
    const root = await render({
      kind: 'run',
      key: 'run-2',
      sentence: 'Ran 1 command (1 failed)',
      items: [CALL],
    });
    const run = root.querySelector('[data-testid="thread-run"]');
    expect(root.querySelector('[data-testid="thread-run-summary"]')?.textContent.trim()).toBe(
      'Ran 1 command (1 failed)',
    );
    const details = run?.querySelector('details');
    expect(details?.open).toBe(false);
    expect(run?.querySelector('[data-testid="thread-receipt-summary"]')?.textContent).toContain(
      'bash: exit 3',
    );
    // Its words say how it went: no status of its own.
    expect(run?.querySelector('.icon')).toBeNull();
    await expectAccessible(root);
  });

  it('keeps a run’s room until the server has told it, named for screen readers', async () => {
    const root = await render({ kind: 'run', key: 'run-2', sentence: null, items: [CALL] });
    const count = root.querySelector('[data-testid="thread-run-summary"] .count');
    expect(count?.classList).toContain('pending');
    expect(count?.textContent.trim()).toBe('Steps');
    expect(root.querySelector('[data-testid="thread-receipt-summary"]')?.textContent).toContain(
      'bash: exit 3',
    );
    await expectAccessible(root);
  });

  it('shows what the model said along the way as plainly as a reply, with what it drew on', async () => {
    const root = await render({
      kind: 'item',
      key: 'm4',
      item: SAID,
      interim: true,
      sources: {
        reply: 4,
        sentence: 'Sources: 1 page',
        pages: [{ url: 'https://tokio.rs/blog', title: 'Tokio blog' }],
      },
    });
    const message = root.querySelector('[data-testid="thread-message"]');
    expect(message?.textContent).toContain('It fails; looking closer.');
    expect(message?.getAttribute('aria-label')).toBe('Update');
    expect(root.querySelector('[data-testid="reply-sources"]')).not.toBeNull();
    await expectAccessible(root);
  });

  it('shows the images a run’s calls made under it, in view while it is closed', async () => {
    const made: CallItem = {
      ...CALL,
      tool: 'image_generate',
      summary: 'image_generate: a red fox at dawn',
      status: 'done',
      images: [{ sha256: 'f'.repeat(64), mediaType: 'image/png', name: 'image.png' }],
    };
    const root = await render({
      kind: 'run',
      key: 'run-2',
      sentence: 'Made 1 image',
      items: [made],
    });
    const figure = root.querySelector('[data-testid="thread-image"]');
    expect(figure?.closest('details')).toBeNull();
    expect(figure?.querySelector('[role="img"]')?.getAttribute('aria-label')).toBe(
      'image_generate: a red fox at dawn',
    );
  });
});
