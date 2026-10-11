import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzClickOutsideDirective } from './click-outside.directive';

@Component({
  imports: [GlzClickOutsideDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div data-testid="panel" (glzClickOutside)="outside.set(outside() + 1)">
      <span data-testid="inside">In</span>
    </div>
    <span data-testid="elsewhere">Out</span>
  `,
})
class HostComponent {
  readonly outside = signal(0);
}

describe('GlzClickOutsideDirective', () => {
  it('emits for presses outside its element only', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const press = (testId: string): void => {
      root
        .querySelector(`[data-testid="${testId}"]`)
        ?.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    };
    press('inside');
    expect(fixture.componentInstance.outside()).toBe(0);
    press('elsewhere');
    expect(fixture.componentInstance.outside()).toBe(1);
  });
});
