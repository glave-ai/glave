import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { MessageTextComponent } from './message-text.component';

async function render(
  text: string,
): Promise<ReturnType<typeof TestBed.createComponent<MessageTextComponent>>> {
  const fixture = TestBed.createComponent(MessageTextComponent);
  fixture.componentRef.setInput('text', text);
  await fixture.whenStable();
  return fixture;
}

describe('MessageTextComponent', () => {
  it('shows short text whole, with no button', async () => {
    const root: HTMLElement = (await render('Make the tests pass')).nativeElement;
    expect(root.querySelector('.text')?.textContent).toBe('Make the tests pass');
    expect(root.querySelector('.folded')).toBeNull();
    expect(root.querySelector('[data-testid="message-text-toggle"]')).toBeNull();
  });

  it('never folds text that is not foldable', async () => {
    const fixture = TestBed.createComponent(MessageTextComponent);
    fixture.componentRef.setInput('text', 'x'.repeat(2000));
    fixture.componentRef.setInput('foldable', false);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('.folded')).toBeNull();
    expect(root.querySelector('[data-testid="message-text-toggle"]')).toBeNull();
  });

  it('folds long text behind a disclosure: Show more, then Show less', async () => {
    const lines = Array.from({ length: 30 }, (_, line) => `Line ${String(line)}`);
    const fixture = await render(lines.join('\n'));
    const root: HTMLElement = fixture.nativeElement;
    const text = root.querySelector<HTMLElement>('.text');
    const toggle = root.querySelector<HTMLButtonElement>('[data-testid="message-text-toggle"]');
    expect(text?.classList).toContain('folded');
    expect(toggle?.textContent.trim()).toBe('Show more');
    expect(toggle?.getAttribute('aria-expanded')).toBe('false');
    expect(toggle?.getAttribute('aria-controls')).toBe(text?.id);
    await expectAccessible(root);
    toggle?.click();
    await fixture.whenStable();
    expect(text?.classList).not.toContain('folded');
    expect(toggle?.textContent.trim()).toBe('Show less');
    expect(toggle?.getAttribute('aria-expanded')).toBe('true');
    toggle?.click();
    await fixture.whenStable();
    expect(text?.classList).toContain('folded');
  });
});
