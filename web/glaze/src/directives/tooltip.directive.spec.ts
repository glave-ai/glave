import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../testing/axe';
import { popoverOpen } from '../testing/platform';
import { GlzTooltipDirective } from './tooltip.directive';

@Component({
  imports: [GlzTooltipDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<button type="button" data-testid="copy" [glzTooltip]="hint()">Copy</button>`,
})
class HostComponent {
  readonly hint = signal('Copy the link');
}

function button(fixture: { nativeElement: unknown }): HTMLButtonElement {
  const found = (fixture.nativeElement as HTMLElement).querySelector<HTMLButtonElement>(
    '[data-testid="copy"]',
  );
  if (found === null) {
    throw new Error('no button');
  }
  return found;
}

function tooltipOf(element: HTMLElement): HTMLElement | null {
  return document.getElementById(element.getAttribute('aria-describedby') ?? '');
}

describe('GlzTooltipDirective', () => {
  it('describes its element with a tooltip popover', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const tooltip = tooltipOf(button(fixture));
    expect(tooltip?.textContent).toBe('Copy the link');
    expect(tooltip?.getAttribute('role')).toBe('tooltip');
    expect(tooltip?.getAttribute('popover')).toBe('manual');
    await expectAccessible(fixture.nativeElement);
  });

  it('shows on hover and on focus, and hides when left or on Escape', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const copy = button(fixture);
    const tooltip = tooltipOf(copy);
    copy.dispatchEvent(new Event('mouseenter'));
    expect(popoverOpen(tooltip)).toBe(true);
    copy.dispatchEvent(new Event('mouseleave'));
    expect(popoverOpen(tooltip)).toBe(false);
    copy.dispatchEvent(new FocusEvent('focusin'));
    expect(popoverOpen(tooltip)).toBe(true);
    expect(tooltip?.style.getPropertyValue('position')).toBe('fixed');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(popoverOpen(tooltip)).toBe(false);
  });

  it('follows its text and leaves with its element', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    fixture.componentInstance.hint.set('Copied');
    await fixture.whenStable();
    const tooltip = tooltipOf(button(fixture));
    expect(tooltip?.textContent).toBe('Copied');
    fixture.destroy();
    expect(tooltip?.isConnected).toBe(false);
  });
});
