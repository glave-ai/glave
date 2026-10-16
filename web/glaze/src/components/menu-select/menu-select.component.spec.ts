import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { type GlzMenuOption, GlzMenuSelectComponent } from './menu-select.component';

const MODELS: readonly GlzMenuOption[] = [
  { value: 'gpt-6-luna', label: 'GPT-6 Luna', hint: 'Default' },
  { value: 'hope-12b', label: 'Hope' },
];

@Component({
  imports: [GlzMenuSelectComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-select
    testId="model"
    label="Model"
    [options]="options"
    [(value)]="value"
  />`,
})
class HostComponent {
  readonly options = MODELS;
  readonly value = signal<string | null>('gpt-6-luna');
}

function trigger(root: HTMLElement): HTMLButtonElement {
  const found = root.querySelector<HTMLButtonElement>('[data-testid="model-trigger"]');
  if (found === null) {
    throw new Error('no trigger');
  }
  return found;
}

describe('GlzMenuSelectComponent', () => {
  afterEach(() => {
    document.body.click();
  });

  it('shows the choice on a button named for what it chooses', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root).textContent.replace(/\s+/gu, ' ').trim()).toBe('Model: GPT-6 Luna');
    expect(trigger(root).getAttribute('aria-haspopup')).toBe('menu');
    await expectAccessible(root);
  });

  it('opens a menu of radio items, the choice checked, and picks one', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    trigger(fixture.nativeElement as HTMLElement).click();
    await fixture.whenStable();
    const menu = document.querySelector('[role="menu"]');
    expect(menu?.getAttribute('aria-label')).toBe('Model');
    const items = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitemradio"]'));
    expect(items.map((item) => item.getAttribute('aria-checked'))).toEqual(['true', 'false']);
    expect(items[0]?.textContent).toContain('Default');
    if (menu !== null) {
      await expectAccessible(menu);
    }
    items[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('hope-12b');
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(trigger(fixture.nativeElement as HTMLElement).textContent).toContain('Hope');
  });

  it('shows the placeholder while nothing is chosen', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.value.set(null);
    await fixture.whenStable();
    expect(trigger(fixture.nativeElement as HTMLElement).textContent).toContain('Choose');
  });
});
