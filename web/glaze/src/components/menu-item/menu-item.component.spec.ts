import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Trash2 } from 'lucide';

import { GlzMenuComponent } from '../menu/menu.component';
import { GlzMenuItemComponent } from './menu-item.component';

@Component({
  imports: [GlzMenuComponent, GlzMenuItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-menu aria-label="Actions">
      <button glzMenuItem tone="danger" [icon]="trash" (triggered)="removed.set(true)">
        Delete
      </button>
    </glz-menu>
  `,
})
class HostComponent {
  protected readonly trash = Trash2;
  readonly removed = signal(false);
}

describe('GlzMenuItemComponent', () => {
  it('runs its action when chosen, with its icon and tone', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const item = (fixture.nativeElement as HTMLElement).querySelector<HTMLElement>(
      '[role="menuitem"]',
    );
    expect(item?.querySelector('svg')).not.toBeNull();
    expect(item?.classList).toContain('glz-tone-danger');
    item?.click();
    expect(fixture.componentInstance.removed()).toBe(true);
  });
});
