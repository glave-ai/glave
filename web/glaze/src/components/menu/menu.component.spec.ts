import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzMenuItemComponent } from '../menu-item/menu-item.component';
import { GlzMenuComponent } from './menu.component';

@Component({
  imports: [GlzMenuComponent, GlzMenuItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-menu aria-label="Actions">
      <button glzMenuItem>Rename</button>
      <button glzMenuItem>Archive</button>
    </glz-menu>
  `,
})
class HostComponent {}

describe('GlzMenuComponent', () => {
  it('is a menu that moves between items with arrow keys', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[role="menu"]')).not.toBeNull();
    const items = root.querySelectorAll<HTMLElement>('[role="menuitem"]');
    expect(items).toHaveLength(2);
    const down = (): void => {
      items[0]?.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'ArrowDown', keyCode: 40, bubbles: true }),
      );
    };
    down();
    expect(document.activeElement).toBe(items[0]);
    down();
    expect(document.activeElement).toBe(items[1]);
    await expectAccessible(root);
  });
});
