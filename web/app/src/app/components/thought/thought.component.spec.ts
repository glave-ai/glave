import { ApplicationRef } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { NO_THOUGHT, type Thought } from '../../lib/conversation';
import { ThoughtComponent } from './thought.component';

async function render(thought: Thought, live = false): Promise<HTMLElement> {
  const fixture = TestBed.createComponent(ThoughtComponent);
  fixture.componentRef.setInput('thought', thought);
  fixture.componentRef.setInput('text', 'The port is taken.\nBy vite.');
  fixture.componentRef.setInput('live', live);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

function label(root: HTMLElement): string | undefined {
  return root.querySelector('[data-testid="thread-thought-label"]')?.textContent;
}

describe('ThoughtComponent', () => {
  it('says how long the model thought, as the server says, and its headline', async () => {
    const root = await render({ headline: 'The port is 5173.', ms: 4250 });
    expect(label(root)).toBe('Thought for 4s');
    expect(root.querySelector('[data-testid="thread-thought-headline"]')?.textContent).toBe(
      'The port is 5173.',
    );
    await expectAccessible(root);
  });

  it('says the model thinks while its reasoning streams, and shimmers', async () => {
    const root = await render({ headline: 'Checking.', ms: null }, true);
    expect(label(root)).toBe('Thinking');
    expect(root.querySelector('[data-testid="thread-thought-line"]')?.classList).toContain('live');
  });

  it('says only that the model thought when the server never said for how long', async () => {
    const root = await render({ headline: 'Checked it.', ms: null });
    expect(label(root)).toBe('Thought');
    expect(root.querySelector('[data-testid="thread-thought-headline"]')?.textContent).toBe(
      'Checked it.',
    );
    await expectAccessible(root);
  });

  it('opens its reasoning in full under the line, and shuts it again', async () => {
    const root = await render(NO_THOUGHT);
    const line = root.querySelector<HTMLElement>('[data-testid="thread-thought-line"]');
    expect(line?.getAttribute('aria-expanded')).toBe('false');
    line?.click();
    await TestBed.inject(ApplicationRef).whenStable();
    expect(line?.getAttribute('aria-expanded')).toBe('true');
    expect(root.querySelector('[data-testid="thread-thought-text"]')?.textContent).toBe(
      'The port is taken.\nBy vite.',
    );
    await expectAccessible(root);
    line?.click();
    await TestBed.inject(ApplicationRef).whenStable();
    expect(root.querySelector('[data-testid="thread-thought-text"]')).toBeNull();
  });
});
