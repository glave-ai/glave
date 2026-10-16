import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzPopoverTriggerDirective } from '../../directives/popover-trigger.directive';
import { expectAccessible } from '../../testing/axe';
import { popoverOpen } from '../../testing/platform';
import { GlzNavItemComponent } from '../nav-item/nav-item.component';
import { GlzPopoverComponent } from './popover.component';

@Component({
  imports: [GlzNavItemComponent, GlzPopoverComponent, GlzPopoverTriggerDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <nav aria-label="You">
      <button glzNavItem data-testid="open" [glzPopoverTrigger]="mine">You</button>
      <glz-popover #mine testId="mine">
        <a glzNavItem href="#settings" data-testid="settings">Settings</a>
        <button glzNavItem data-testid="sign-out">Sign out</button>
      </glz-popover>
    </nav>
  `,
})
class HostComponent {}

async function render(): Promise<{
  readonly root: HTMLElement;
  readonly button: HTMLButtonElement;
  readonly panel: HTMLElement;
  readonly settle: () => Promise<void>;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  const root: HTMLElement = fixture.nativeElement;
  const button = root.querySelector<HTMLButtonElement>('[data-testid="open"]');
  const panel = root.querySelector<HTMLElement>('[data-testid="mine"]');
  if (button === null || panel === null) {
    throw new Error('not rendered');
  }
  return { root, button, panel, settle: () => fixture.whenStable() };
}

describe('GlzPopoverComponent', () => {
  it('is a popover its button opens natively, and says it is closed', async () => {
    const { root, button, panel } = await render();
    expect(panel.getAttribute('popover')).toBe('auto');
    expect(button.popoverTargetElement).toBe(panel);
    expect(button.getAttribute('aria-controls')).toBe(panel.id);
    expect(button.getAttribute('aria-expanded')).toBe('false');
    await expectAccessible(root);
  });

  it('says it is open, and closes once a link in it is followed', async () => {
    const { root, button, panel, settle } = await render();
    panel.showPopover();
    await settle();
    expect(button.getAttribute('aria-expanded')).toBe('true');
    await expectAccessible(root);
    root.querySelector<HTMLElement>('[data-testid="settings"]')?.click();
    await settle();
    expect(popoverOpen(panel)).toBe(false);
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });

  it('closes once a button in it is pressed, and stays open for a click elsewhere in it', async () => {
    const { root, panel, settle } = await render();
    panel.showPopover();
    await settle();
    panel.click();
    expect(popoverOpen(panel)).toBe(true);
    root.querySelector<HTMLElement>('[data-testid="sign-out"]')?.click();
    await settle();
    expect(popoverOpen(panel)).toBe(false);
  });
});
