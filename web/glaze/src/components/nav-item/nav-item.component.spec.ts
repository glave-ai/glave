import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Hash } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { GlzNavItemComponent } from './nav-item.component';

@Component({
  imports: [GlzNavItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <a glzNavItem href="/t/1" current [icon]="icon"
      >Plan the launch <span glzNavItemEnd>running</span></a
    >
    <button glzNavItem type="button"><span glzNavItemStart>A</span> Ada</button>
    <a glzNavItem href="/t/2">Fix the build</a>
  `,
})
class HostComponent {
  protected readonly icon = Hash;
}

describe('GlzNavItemComponent', () => {
  it('shows icon, text and end, and marks the current page', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const link = (fixture.nativeElement as HTMLElement).querySelector('a');
    expect(link?.getAttribute('aria-current')).toBe('page');
    expect(link?.querySelector('.text')?.textContent.trim()).toBe('Plan the launch');
    expect(link?.querySelector('.end')?.textContent).toBe('running');
    expect(link?.querySelector('svg')).not.toBeNull();
  });

  it('marks the current page by its background alone, at the weight of the rest', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const [current, other] = Array.from(root.querySelectorAll('a'));
    expect(current?.getAttribute('aria-current')).toBe('page');
    expect(other?.hasAttribute('aria-current')).toBe(false);
    const weight = (element: Element | undefined): string =>
      element === undefined ? 'none' : getComputedStyle(element).fontWeight;
    expect(weight(current)).toBe(weight(other));
    await expectAccessible(root);
  });

  it('puts what leads it before its text', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const button = (fixture.nativeElement as HTMLElement).querySelector('button');
    expect(button?.firstElementChild?.textContent).toBe('A');
    expect(button?.querySelector('.text')?.textContent.trim()).toBe('Ada');
  });
});
