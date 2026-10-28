import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { NO_THOUGHT, type Thought } from '../../lib/conversation';
import { ThoughtService } from '../../services/thought.service';
import { ThoughtComponent } from './thought.component';

/** The thread's choice of reasoning to show in full, as the test sets it. */
class FakeThoughts {
  readonly selected = signal<string | null>(null);
  readonly chosen: string[] = [];

  choose(key: string): void {
    this.chosen.push(key);
    this.selected.set(key);
  }
}

async function render(
  thought: Thought,
  live = false,
): Promise<{ readonly root: HTMLElement; readonly thoughts: FakeThoughts }> {
  const thoughts = new FakeThoughts();
  TestBed.configureTestingModule({
    providers: [{ provide: ThoughtService, useValue: thoughts }],
  });
  const fixture = TestBed.createComponent(ThoughtComponent);
  fixture.componentRef.setInput('key', 'message-3');
  fixture.componentRef.setInput('thought', thought);
  fixture.componentRef.setInput('live', live);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, thoughts };
}

function label(root: HTMLElement): string | undefined {
  return root.querySelector('[data-testid="thread-thought-label"]')?.textContent;
}

describe('ThoughtComponent', () => {
  it('says how long the model thought, as the server says, and its headline', async () => {
    const { root } = await render({ headline: 'The port is 5173.', ms: 4250 });
    expect(label(root)).toBe('Thought for 4s');
    expect(root.querySelector('[data-testid="thread-thought-headline"]')?.textContent).toBe(
      'The port is 5173.',
    );
    await expectAccessible(root);
  });

  it('says the model thinks while its reasoning streams, and shimmers', async () => {
    const { root } = await render({ headline: 'Checking.', ms: null }, true);
    expect(label(root)).toBe('Thinking');
    expect(root.querySelector('[data-testid="thread-thought-line"]')?.classList).toContain('live');
  });

  it('says only that the model thought when the server never said for how long', async () => {
    const { root } = await render(NO_THOUGHT);
    expect(label(root)).toBe('Thought');
    expect(root.querySelector('[data-testid="thread-thought-headline"]')).toBeNull();
    await expectAccessible(root);
  });

  it('asks to show its reasoning in full, and says when it shows', async () => {
    const { root, thoughts } = await render({ headline: 'Done.', ms: 1000 });
    const line = root.querySelector<HTMLElement>('[data-testid="thread-thought-line"]');
    expect(line?.getAttribute('aria-pressed')).toBe('false');
    line?.click();
    expect(thoughts.chosen).toEqual(['message-3']);
    TestBed.tick();
    expect(line?.getAttribute('aria-pressed')).toBe('true');
    await expectAccessible(root);
  });
});
