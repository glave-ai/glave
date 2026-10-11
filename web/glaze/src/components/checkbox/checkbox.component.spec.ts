import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import { FormField, form, required } from '@angular/forms/signals';

import { expectAccessible } from '../../testing/axe';
import { GlzErrorComponent } from '../error/error.component';
import { GlzFormFieldComponent } from '../form-field/form-field.component';
import { GlzCheckboxComponent } from './checkbox.component';

@Component({
  imports: [GlzCheckboxComponent, GlzFormFieldComponent, GlzErrorComponent, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Terms">
      <glz-checkbox testId="checkbox" [formControl]="agree">I agree</glz-checkbox>
      <glz-error>Agree to go on.</glz-error>
    </glz-form-field>
  `,
})
class ReactiveHostComponent {
  readonly agree = new FormControl(false, { validators: [Validators.requiredTrue] });
}

@Component({
  imports: [GlzCheckboxComponent, FormField],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-checkbox testId="checkbox" [formField]="settings.remember"
    >Remember</glz-checkbox
  >`,
})
class SignalHostComponent {
  readonly model = signal({ remember: true });
  readonly settings = form(this.model, (path) => {
    required(path.remember);
  });
}

@Component({
  imports: [GlzCheckboxComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-checkbox testId="checkbox" [(checked)]="on" indeterminate disabled
    >All</glz-checkbox
  >`,
})
class ModelHostComponent {
  readonly on = signal(false);
}

function box(root: HTMLElement): HTMLInputElement {
  const input = root.querySelector<HTMLInputElement>('[data-testid="checkbox-input"]');
  if (input === null) {
    throw new Error('no checkbox');
  }
  return input;
}

describe('GlzCheckboxComponent', () => {
  it('binds to reactive forms and shows its error once touched', async () => {
    const fixture = TestBed.createComponent(ReactiveHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const input = box(root);
    expect(input.closest('label')?.textContent).toContain('I agree');
    input.dispatchEvent(new Event('blur'));
    await fixture.whenStable();
    expect(root.textContent).toContain('Agree to go on.');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    input.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.agree.value).toBe(true);
    expect(root.textContent).not.toContain('Agree to go on.');
    fixture.componentInstance.agree.setValue(false);
    await fixture.whenStable();
    expect(input.checked).toBe(false);
    await expectAccessible(root);
  });

  it('binds to signal forms', async () => {
    const fixture = TestBed.createComponent(SignalHostComponent);
    await fixture.whenStable();
    const input = box(fixture.nativeElement);
    expect(input.checked).toBe(true);
    input.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.model().remember).toBe(false);
  });

  it('shows indeterminate and disabled states', async () => {
    const fixture = TestBed.createComponent(ModelHostComponent);
    await fixture.whenStable();
    const input = box(fixture.nativeElement);
    expect(input.indeterminate).toBe(true);
    expect(input.disabled).toBe(true);
  });
});
