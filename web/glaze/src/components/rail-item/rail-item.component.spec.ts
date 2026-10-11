import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Brain } from 'lucide';

import { GlzRailItemComponent } from './rail-item.component';

@Component({
  imports: [GlzRailItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<a glzRailItem href="/memory" label="Memory" [icon]="icon" [current]="here()"></a>`,
})
class HostComponent {
  protected readonly icon = Brain;
  readonly here = signal(false);
}

describe('GlzRailItemComponent', () => {
  it('shows its icon and label, and marks the current destination', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const link = (fixture.nativeElement as HTMLElement).querySelector('a');
    expect(link?.textContent.trim()).toBe('Memory');
    expect(link?.querySelector('svg')).not.toBeNull();
    expect(link?.hasAttribute('aria-current')).toBe(false);
    fixture.componentInstance.here.set(true);
    await fixture.whenStable();
    expect(link?.getAttribute('aria-current')).toBe('page');
  });
});
