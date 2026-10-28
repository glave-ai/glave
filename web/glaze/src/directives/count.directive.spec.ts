import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../testing/axe';
import { fakeMotion } from '../testing/motion';
import { GlzCountDirective } from './count.directive';

@Component({
  imports: [GlzCountDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<p>Lines: <span [glzCount]="lines()" [glzCountFormat]="plus"></span></p>`,
})
class HostComponent {
  readonly lines = signal(1200);
  private readonly numbers = new Intl.NumberFormat('en');
  readonly plus = (value: number): string => `+${this.numbers.format(value)}`;
}

async function render(): Promise<{
  readonly count: () => string | null | undefined;
  readonly set: (lines: number) => Promise<unknown>;
  readonly root: HTMLElement;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  const root = fixture.nativeElement as HTMLElement;
  return {
    root,
    count: () => root.querySelector('.glz-count')?.textContent,
    set: (lines) => {
      fixture.componentInstance.lines.set(lines);
      return fixture.whenStable();
    },
  };
}

describe('GlzCountDirective', () => {
  it('shows its first number as it is, and counts up from it as it grows', async () => {
    const motion = fakeMotion('320ms');
    const { root, count, set } = await render();
    expect(count()).toBe('+1,200');
    await set(1300);
    // It starts from what it showed.
    expect(count()).toBe('+1,200');
    motion.to(0.5);
    expect(count()).toBe('+1,250');
    // Grown again on the way: on from where it got to.
    await set(1450);
    motion.to(0.5);
    expect(count()).toBe('+1,350');
    motion.end();
    expect(count()).toBe('+1,450');
    await expectAccessible(root);
    motion.restore();
  });

  it('shows a smaller number at once', async () => {
    const motion = fakeMotion('320ms');
    const { count, set } = await render();
    await set(900);
    expect(count()).toBe('+900');
    motion.restore();
  });

  it('shows every number at once when the person asks for less motion', async () => {
    const motion = fakeMotion('0ms');
    const { count, set } = await render();
    await set(1300);
    expect(count()).toBe('+1,300');
    motion.restore();
  });
});
