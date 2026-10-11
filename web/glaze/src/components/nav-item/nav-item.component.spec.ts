import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Hash } from 'lucide';

import { GlzNavItemComponent } from './nav-item.component';

@Component({
  imports: [GlzNavItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <a glzNavItem href="/t/1" current [icon]="icon"
      >Plan the launch <span glzNavItemEnd>running</span></a
    >
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
});
