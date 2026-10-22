import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../testing/axe';
import { GlzFocusOnPressDirective } from './focus-on-press.directive';

@Component({
  imports: [GlzFocusOnPressDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <input data-testid="field" aria-label="Message" />
    <button type="button" glzFocusOnPress data-testid="model">Model</button>
  `,
})
class HostComponent {}

async function rendered(): Promise<{ readonly field: HTMLElement; readonly button: HTMLElement }> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  const root: HTMLElement = fixture.nativeElement;
  const field = root.querySelector<HTMLElement>('[data-testid="field"]');
  const button = root.querySelector<HTMLElement>('[data-testid="model"]');
  if (field === null || button === null) {
    throw new Error('not rendered');
  }
  await expectAccessible(root);
  return { field, button };
}

function pressed(button: HTMLElement, which: number): MouseEvent {
  const event = new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: which });
  button.dispatchEvent(event);
  return event;
}

describe('GlzFocusOnPressDirective', () => {
  it('takes focus to its button as it is pressed, and keeps the browser from moving it', async () => {
    const { field, button } = await rendered();
    field.focus();
    expect(pressed(button, 0).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(button);
  });

  it('leaves a press of another mouse button alone', async () => {
    const { field, button } = await rendered();
    field.focus();
    expect(pressed(button, 2).defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(field);
  });
});
