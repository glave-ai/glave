import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';

import { GlzCheckableDirective } from './checkable.directive';

/** The smallest control built on the directive: one native checkbox. */
@Component({
  selector: 'glz-test-checkable',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<input
    type="checkbox"
    aria-label="On"
    [checked]="checked()"
    [disabled]="isDisabled()"
    [attr.aria-invalid]="ariaInvalid()"
    [attr.aria-required]="ariaRequired()"
    (change)="changed($event)"
    (blur)="blurred()"
  />`,
})
class CheckComponent extends GlzCheckableDirective {}

@Component({
  imports: [CheckComponent, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-test-checkable data-testid="form" [formControl]="agree" />
    <glz-test-checkable data-testid="model" [(checked)]="on" (touch)="touched.set(true)" />
  `,
})
class HostComponent {
  readonly agree = new FormControl(false, { validators: [Validators.requiredTrue] });
  readonly on = signal(false);
  readonly touched = signal(false);
}

function box(root: HTMLElement, testId: string): HTMLInputElement {
  const input = root.querySelector<HTMLInputElement>(`[data-testid="${testId}"] input`);
  if (input === null) {
    throw new Error(`no ${testId}`);
  }
  return input;
}

describe('GlzCheckableDirective', () => {
  it('binds a form both ways, shows its state and follows disabling', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const input = box(root, 'form');
    input.dispatchEvent(new Event('blur'));
    await fixture.whenStable();
    expect(input.getAttribute('aria-invalid')).toBe('true');
    input.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.agree.value).toBe(true);
    expect(input.hasAttribute('aria-invalid')).toBe(false);
    fixture.componentInstance.agree.setValue(false);
    fixture.componentInstance.agree.disable();
    await fixture.whenStable();
    expect(input.checked).toBe(false);
    expect(input.disabled).toBe(true);
  });

  it('binds a model and says when it is touched', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const input = box(fixture.nativeElement as HTMLElement, 'model');
    input.click();
    input.dispatchEvent(new Event('blur'));
    await fixture.whenStable();
    expect(fixture.componentInstance.on()).toBe(true);
    expect(fixture.componentInstance.touched()).toBe(true);
  });
});
