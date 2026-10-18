import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { GlzOption } from '../../lib/options';
import { expectAccessible } from '../../testing/axe';
import { GlzMenuButtonComponent } from './menu-button.component';

const SORTS: readonly GlzOption[] = [
  { value: 'recent', label: 'Recent activity' },
  { value: 'name', label: 'Name' },
];

@Component({
  imports: [GlzMenuButtonComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-button
    testId="more"
    label="Options for glave"
    choiceLabel="Sort by"
    [actions]="actions"
    [choices]="sorts"
    [(choice)]="sort"
    (action)="pick($event)"
  />`,
})
class HostComponent {
  readonly actions: readonly GlzOption[] = [{ value: 'rename', label: 'Rename' }];
  readonly sorts = SORTS;
  readonly sort = signal<string | null>('recent');
  readonly picked: string[] = [];

  pick(value: string): void {
    this.picked.push(value);
  }
}

@Component({
  imports: [GlzMenuButtonComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-menu-button
    testId="chats"
    label="Options for Chats"
    choiceLabel="Sort by"
    [choices]="sorts"
    choice="name"
  />`,
})
class ChoiceOnlyComponent {
  readonly sorts = SORTS;
}

function trigger(root: HTMLElement, testId: string): HTMLButtonElement {
  const found = root.querySelector<HTMLButtonElement>(`[data-testid="${testId}-trigger"]`);
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
    const button = trigger(root, 'more');
    expect(button.getAttribute('aria-label')).toBe('Options for glave');
    expect(button.getAttribute('aria-haspopup')).toBe('menu');
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(button.querySelector('glz-icon')).not.toBeNull();
    await expectAccessible(root);
  });

  it('opens its actions, then the choice under its label, and emits the action picked', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    trigger(fixture.nativeElement as HTMLElement, 'more').click();
    await fixture.whenStable();
    const menu = document.querySelector('[role="menu"]');
    expect(menu?.getAttribute('aria-label')).toBe('Options for glave');
    expect(
      shown('more-action').map((item) => [item.getAttribute('role'), item.textContent.trim()]),
    ).toEqual([['menuitem', 'Rename']]);
    const group = menu?.querySelector('[role="group"]');
    expect(document.getElementById(group?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      'Sort by',
    );
    expect(shown('more-choice').map((item) => item.getAttribute('aria-checked'))).toEqual([
      'true',
      'false',
    ]);
    if (menu !== null) {
      await expectAccessible(menu);
    }
    shown('more-action')[0]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.picked).toEqual(['rename']);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it('picks a choice, which it shows checked the next time', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    trigger(root, 'more').click();
    await fixture.whenStable();
    shown('more-choice')[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.sort()).toBe('name');
    expect(fixture.componentInstance.picked).toEqual([]);
    trigger(root, 'more').click();
    await fixture.whenStable();
    expect(shown('more-choice').map((item) => item.getAttribute('aria-checked'))).toEqual([
      'false',
      'true',
    ]);
  });

  it('with only a choice, has no actions', async () => {
    const fixture = TestBed.createComponent(ChoiceOnlyComponent);
    await fixture.whenStable();
    trigger(fixture.nativeElement as HTMLElement, 'chats').click();
    await fixture.whenStable();
    const menu = document.querySelector('[role="menu"]');
    expect(shown('chats-action')).toHaveLength(0);
    expect(shown('chats-choice').map((item) => item.textContent.trim())).toEqual([
      'Recent activity',
      'Name',
    ]);
    if (menu !== null) {
      await expectAccessible(menu);
    }
  });
});
