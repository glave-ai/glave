import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { GlzOption } from '../../lib/options';
import { expectAccessible } from '../../testing/axe';
import { GlzMenuButtonComponent } from './menu-button.component';

@Component({
  imports: [GlzMenuButtonComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-button
    testId="more"
    label="Options for Atlas"
    [actions]="actions"
    (action)="pick($event)"
  />`,
})
class HostComponent {
  readonly actions: readonly GlzOption[] = [
    { value: 'rename', label: 'Rename' },
    { value: 'remove', label: 'Remove' },
  ];
  readonly picked: string[] = [];

  pick(value: string): void {
    this.picked.push(value);
  }
}

function trigger(root: HTMLElement): HTMLButtonElement {
  const found = root.querySelector<HTMLButtonElement>('[data-testid="more-trigger"]');
  if (found === null) {
    throw new Error('no trigger');
  }
  return found;
}

function shown(testId: string): readonly HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`));
}

describe('GlzMenuButtonComponent', () => {
  afterEach(() => {
    document.body.click();
  });

  it('is an icon button named for what its menu is for', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const button = trigger(root);
    expect(button.getAttribute('aria-label')).toBe('Options for Atlas');
    expect(button.getAttribute('aria-haspopup')).toBe('menu');
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(button.querySelector('glz-icon')).not.toBeNull();
    await expectAccessible(root);
  });

  it('opens its actions in order, and emits the one picked', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    trigger(fixture.nativeElement as HTMLElement).click();
    await fixture.whenStable();
    const menu = document.querySelector('[role="menu"]');
    expect(menu?.getAttribute('aria-label')).toBe('Options for Atlas');
    expect(
      shown('more-action').map((item) => [item.getAttribute('role'), item.textContent.trim()]),
    ).toEqual([
      ['menuitem', 'Rename'],
      ['menuitem', 'Remove'],
    ]);
    if (menu !== null) {
      await expectAccessible(menu);
    }
    shown('more-action')[0]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.picked).toEqual(['rename']);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});
