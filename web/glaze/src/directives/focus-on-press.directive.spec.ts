import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../testing/axe';
import { GlzFocusOnPressDirective } from './focus-on-press.directive';

@Component({
  imports: [GlzFocusOnPressDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <input data-testid="field" aria-label="Message" />
    <button type="button" glzFocusOnPress data-testid="model" [attr.aria-expanded]="open()">
      Model
    </button>
  `,
})
class HostComponent {
  /** Whether the button's menu is open. */
  readonly open = signal(true);
}

async function rendered(open: boolean): Promise<{
  readonly field: HTMLElement;
  readonly button: HTMLElement;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  fixture.componentInstance.open.set(open);
  await fixture.whenStable();
  const root: HTMLElement = fixture.nativeElement;
  const field = root.querySelector<HTMLElement>('[data-testid="field"]');
  const button = root.querySelector<HTMLElement>('[data-testid="model"]');
  if (field === null || button === null) {
    throw new Error('not rendered');
  }
  await expectAccessible(root);
  field.focus();
  return { field, button };
}

function pressed(button: HTMLElement, which: number): MouseEvent {
  const event = new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: which });
  button.dispatchEvent(event);
  return event;
}

describe('GlzFocusOnPressDirective', () => {
  it('takes focus to the button of an open menu as it is pressed, and keeps it there', async () => {
    const { button } = await rendered(true);
    expect(pressed(button, 0).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(button);
  });

  it('leaves a press on the button of a closed menu to the browser', async () => {
    const { field, button } = await rendered(false);
    expect(pressed(button, 0).defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(field);
  });

  it('leaves a press of another mouse button alone', async () => {
    const { field, button } = await rendered(true);
    expect(pressed(button, 2).defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(field);
  });
});
