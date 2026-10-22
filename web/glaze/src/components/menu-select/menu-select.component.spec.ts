import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Folder, MessageCircle } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { clickAsSafari } from '../../testing/press';
import { type GlzMenuOption, GlzMenuSelectComponent } from './menu-select.component';

const MODELS: readonly GlzMenuOption[] = [
  { value: 'gpt-6-luna', label: 'GPT-6 Luna', hint: 'Default' },
  { value: 'hope-12b', label: 'Hope' },
];

@Component({
  imports: [GlzMenuSelectComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-select testId="model" label="Model" [options]="options" [(value)]="value" />
    <button type="button" data-testid="elsewhere" (click)="elsewhere()">Elsewhere</button>`,
})
class HostComponent {
  readonly options = MODELS;
  readonly value = signal<string | null>('gpt-6-luna');
  /** How many times the button outside the menu was clicked. */
  readonly clicks = signal(0);

  elsewhere(): void {
    this.clicks.update((count) => count + 1);
  }
}

@Component({
  imports: [GlzMenuSelectComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-select
    testId="where"
    label="Where"
    outlined
    [options]="options"
    [(value)]="value"
  />`,
})
class PlacesComponent {
  readonly options: readonly GlzMenuOption[] = [
    { value: 'me', label: 'A chat', icon: MessageCircle },
    { value: 'p1', label: 'Atlas', icon: Folder },
  ];
  readonly value = signal<string | null>('me');
}

@Component({
  imports: [GlzMenuSelectComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-select
    testId="mode"
    label="Approvals"
    quiet
    [options]="options"
    value="auto"
  />`,
})
class QuietComponent {
  readonly options: readonly GlzMenuOption[] = [
    { value: 'auto', label: 'Auto', short: 'Auto', hint: 'Acts for you', icon: MessageCircle },
  ];
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

  it('closes on its own button, pressed as Safari presses it, and stays closed', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const button = trigger(root);
    await clickAsSafari(button);
    await fixture.whenStable();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    expect(button.getAttribute('aria-expanded')).toBe('true');
    await clickAsSafari(button);
    await fixture.whenStable();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(button.getAttribute('aria-expanded')).toBe('false');
    // The press took focus to the button, so focus never left the menu and its button: in a
    // browser the menu, which closes once it does, stays open until the click closes it (jsdom
    // renders no CDK overlay, so focus can't be in the menu here; end to end it is).
    expect(document.activeElement).toBe(button);
    await expectAccessible(root);
  });

  it('closes on a click outside, and that click still does what it does', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    await clickAsSafari(trigger(root));
    await fixture.whenStable();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    const elsewhere = root.querySelector<HTMLElement>('[data-testid="elsewhere"]');
    expect(elsewhere).not.toBeNull();
    await clickAsSafari(elsewhere ?? root);
    await fixture.whenStable();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(fixture.componentInstance.clicks()).toBe(1);
  });

  it('closes on Escape and gives focus back to its button', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const button = trigger(fixture.nativeElement as HTMLElement);
    button.click();
    await fixture.whenStable();
    document
      .querySelector('[role="menu"]')
      ?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', keyCode: 27, bubbles: true }));
    await fixture.whenStable();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  it('as an outlined chip, shows the choice with its icon, as the menu does', async () => {
    const fixture = TestBed.createComponent(PlacesComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const button = root.querySelector<HTMLButtonElement>('[data-testid="where-trigger"]');
    expect(root.querySelector('glz-menu-select')?.classList).toContain('glz-menu-select--outlined');
    expect(button?.querySelectorAll('glz-icon')).toHaveLength(2);
    expect(button?.textContent.replace(/\s+/gu, ' ').trim()).toBe('Where: A chat');
    await expectAccessible(root);
    button?.click();
    await fixture.whenStable();
    const items = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitemradio"]'));
    expect(items.map((item) => item.querySelectorAll('.icon').length)).toEqual([1, 1]);
    items[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('p1');
  });

  it('as a quiet one, shows its icon and short word, small and muted', async () => {
    const fixture = TestBed.createComponent(QuietComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('glz-menu-select')?.classList).toContain('glz-menu-select--quiet');
    const button = root.querySelector<HTMLButtonElement>('[data-testid="mode-trigger"]');
    expect(button?.textContent.replace(/\s+/gu, ' ').trim()).toBe('Approvals: Auto');
    expect(button?.querySelectorAll('glz-icon')).toHaveLength(2);
    await expectAccessible(root);
  });

  it('shows the placeholder while nothing is chosen', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.value.set(null);
    await fixture.whenStable();
    expect(trigger(fixture.nativeElement as HTMLElement).textContent).toContain('Choose');
  });
});
