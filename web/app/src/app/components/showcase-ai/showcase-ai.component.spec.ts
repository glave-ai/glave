import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseAiComponent } from './showcase-ai.component';

describe('ShowcaseAiComponent', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows a conversation with every part, answered in place', async () => {
    const fixture = TestBed.createComponent(ShowcaseAiComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelectorAll('glz-receipt')).toHaveLength(2);
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="approval-allow-once"]')?.click();
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="approval-decided"]')?.textContent).toBe(
      'Allowed once',
    );
  });

  it('sends from the composer and streams a reply, which can be stopped', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const fixture = TestBed.createComponent(ShowcaseAiComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const input = root.querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]');
    if (input) {
      input.value = 'Hello';
      input.dispatchEvent(new Event('input'));
    }
    await fixture.whenStable();
    root.querySelector<HTMLButtonElement>('[data-testid="composer-send"]')?.click();
    vi.advanceTimersByTime(160);
    await fixture.whenStable();
    const messages = root.querySelectorAll('[data-testid="showcase-message"]');
    expect(messages).toHaveLength(2);
    expect(messages[0]?.textContent).toContain('Hello');
    expect(root.querySelector('[data-testid="composer-stop"]')).not.toBeNull();
    root.querySelector<HTMLButtonElement>('[data-testid="composer-stop"]')?.click();
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="composer-send"]')).not.toBeNull();
  });
});
