import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzMenuItemComponent } from '../components/menu-item/menu-item.component';
import { GlzMenuComponent } from '../components/menu/menu.component';
import { expectAccessible } from '../testing/axe';
import { GlzMenuTriggerDirective } from './menu-trigger.directive';

@Component({
  imports: [GlzMenuTriggerDirective, GlzMenuComponent, GlzMenuItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button type="button" data-testid="trigger" [glzMenuTrigger]="menu">Actions</button>
    <ng-template #menu>
      <glz-menu>
        <button glzMenuItem data-testid="rename" (triggered)="chosen.set('rename')">Rename</button>
        <button glzMenuItem tone="danger" disabled>Delete</button>
      </glz-menu>
    </ng-template>
  `,
})
class HostComponent {
  readonly chosen = signal('');
}

describe('GlzMenuTriggerDirective', () => {
  it('opens a menu whose items run their action and close it', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const trigger = (fixture.nativeElement as HTMLElement).querySelector<HTMLButtonElement>(
      '[data-testid="trigger"]',
    );
    expect(trigger?.getAttribute('aria-haspopup')).toBe('menu');
    trigger?.click();
    await fixture.whenStable();
    expect(trigger?.getAttribute('aria-expanded')).toBe('true');
    const menu = document.querySelector('[role="menu"]');
    expect(menu).not.toBeNull();
    const items = document.querySelectorAll('[role="menuitem"]');
    expect(items).toHaveLength(2);
    expect(items[1]?.getAttribute('aria-disabled')).toBe('true');
    expect(items[1]?.classList).toContain('glz-tone-danger');
    if (menu !== null) {
      await expectAccessible(menu);
    }
    document.querySelector<HTMLButtonElement>('[data-testid="rename"]')?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.chosen()).toBe('rename');
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});
