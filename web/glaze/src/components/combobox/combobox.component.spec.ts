import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule } from '@angular/forms';
import { FormField, form } from '@angular/forms/signals';

import { expectAccessible } from '../../testing/axe';
import { popoverOpen } from '../../testing/platform';
import { GlzFormFieldComponent } from '../form-field/form-field.component';
import { GlzComboboxComponent } from './combobox.component';

const MODELS = [
  { value: 'hope', label: 'Hope' },
  { value: 'luna', label: 'GPT-6 Luna' },
  { value: 'haiku', label: 'Haiku' },
];

@Component({
  imports: [GlzComboboxComponent, GlzFormFieldComponent, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Model">
      <glz-combobox testId="combobox" [options]="models" [formControl]="model" />
    </glz-form-field>
  `,
})
class ReactiveHostComponent {
  readonly models = MODELS;
  readonly model = new FormControl<string | null>('hope');
}

@Component({
  imports: [GlzComboboxComponent, FormField],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-combobox
    testId="combobox"
    label="Model"
    [options]="models"
    [formField]="settings.model"
  />`,
})
class SignalHostComponent {
  readonly models = MODELS;
  readonly model = signal({ model: null as string | null });
  readonly settings = form(this.model);
}

function input(root: HTMLElement): HTMLInputElement {
  const found = root.querySelector<HTMLInputElement>('[data-testid="combobox-input"]');
  if (found === null) {
    throw new Error('no combobox');
  }
  return found;
}

function shownOptions(): readonly HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-testid="combobox-option"]'));
}

const KEY_CODES: Readonly<Record<string, number>> = {
  ArrowDown: 40,
  ArrowUp: 38,
  Enter: 13,
  Escape: 27,
};

function key(target: HTMLElement, name: string): void {
  target.dispatchEvent(
    new KeyboardEvent('keydown', { key: name, keyCode: KEY_CODES[name], bubbles: true }),
  );
}

describe('GlzComboboxComponent', () => {
  it('shows the picked label, filters as you type and picks with the keyboard', async () => {
    const fixture = TestBed.createComponent(ReactiveHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const box = input(root);
    expect(box.value).toBe('Hope');
    expect(root.querySelector('label')?.htmlFor).toBe(box.id);
    expect(box.getAttribute('aria-expanded')).toBe('false');

    box.value = 'h';
    box.dispatchEvent(new Event('input'));
    await fixture.whenStable();
    expect(box.getAttribute('aria-expanded')).toBe('true');
    expect(shownOptions().map((option) => option.textContent.trim())).toEqual(['Hope', 'Haiku']);
    expect(box.getAttribute('aria-activedescendant')).toBe(shownOptions()[0]?.id);

    key(box, 'ArrowDown');
    await fixture.whenStable();
    expect(box.getAttribute('aria-activedescendant')).toBe(shownOptions()[1]?.id);
    await expectAccessible(document.body);
    key(box, 'Enter');
    await fixture.whenStable();
    expect(fixture.componentInstance.model.value).toBe('haiku');
    expect(box.value).toBe('Haiku');
    expect(box.getAttribute('aria-expanded')).toBe('false');
  });

  it('opens on arrow keys with every option, closes on Escape, picks by click', async () => {
    const fixture = TestBed.createComponent(SignalHostComponent);
    await fixture.whenStable();
    const box = input(fixture.nativeElement);
    key(box, 'ArrowDown');
    await fixture.whenStable();
    expect(shownOptions()).toHaveLength(3);
    expect(popoverOpen(document.querySelector('[data-testid="combobox-listbox"]'))).toBe(true);
    key(box, 'Escape');
    await fixture.whenStable();
    expect(box.getAttribute('aria-expanded')).toBe('false');

    box.click();
    await fixture.whenStable();
    shownOptions()[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.model().model).toBe('luna');
    expect(box.value).toBe('GPT-6 Luna');
  });

  it('says when nothing matches, and puts the label back on close', async () => {
    const fixture = TestBed.createComponent(ReactiveHostComponent);
    await fixture.whenStable();
    const box = input(fixture.nativeElement);
    box.value = 'zzz';
    box.dispatchEvent(new Event('input'));
    await fixture.whenStable();
    expect(document.querySelector('[data-testid="combobox-listbox"]')?.textContent).toContain(
      'No matches',
    );
    key(box, 'Escape');
    await fixture.whenStable();
    expect(box.value).toBe('Hope');
  });
});
