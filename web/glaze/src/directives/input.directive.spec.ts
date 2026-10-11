import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import { FormField, form, required } from '@angular/forms/signals';

import { GlzErrorComponent } from '../components/error/error.component';
import { GlzFormFieldComponent } from '../components/form-field/form-field.component';
import { expectAccessible } from '../testing/axe';
import { GlzInputDirective } from './input.directive';

@Component({
  imports: [GlzFormFieldComponent, GlzErrorComponent, GlzInputDirective, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Name" hint="Everyone sees it">
      <input glzInput data-testid="name" [formControl]="name" />
      <glz-error>Give it a name.</glz-error>
    </glz-form-field>
  `,
})
class ReactiveHostComponent {
  readonly name = new FormControl('', { nonNullable: true, validators: [Validators.required] });
}

@Component({
  imports: [GlzFormFieldComponent, GlzInputDirective, FormField],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Title">
      <textarea glzInput data-testid="title" [formField]="draft.title"></textarea>
    </glz-form-field>
  `,
})
class SignalHostComponent {
  readonly model = signal({ title: '' });
  readonly draft = form(this.model, (path) => {
    required(path.title, { message: 'A title, please.' });
  });
}

function element<T extends Element = HTMLElement>(root: HTMLElement, testId: string): T {
  const found = root.querySelector<T>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no element ${testId}`);
  }
  return found;
}

describe('GlzInputDirective', () => {
  it('is labelled and described by its form field, with reactive forms', async () => {
    const fixture = TestBed.createComponent(ReactiveHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const input = element<HTMLInputElement>(root, 'name');
    const label = root.querySelector('label');
    expect(label?.htmlFor).toBe(input.id);
    expect(input.classList).toContain('glz-input');
    expect(input.getAttribute('aria-required')).toBe('true');
    const hint = root.querySelector('.hint');
    expect(input.getAttribute('aria-describedby')).toBe(hint?.id);
    expect(root.textContent).not.toContain('Give it a name.');
    await expectAccessible(root);

    input.dispatchEvent(new Event('blur'));
    await fixture.whenStable();
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(root.textContent).toContain('Give it a name.');
    expect(root.textContent).not.toContain('Everyone sees it');
    const errors = root.querySelector('.errors');
    expect(input.getAttribute('aria-describedby')).toBe(errors?.id);

    input.value = 'Ada';
    input.dispatchEvent(new Event('input'));
    await fixture.whenStable();
    expect(fixture.componentInstance.name.value).toBe('Ada');
    expect(input.hasAttribute('aria-invalid')).toBe(false);
  });

  it('shows signal forms messages by themselves', async () => {
    const fixture = TestBed.createComponent(SignalHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const title = element<HTMLTextAreaElement>(root, 'title');
    expect(root.querySelector('label')?.htmlFor).toBe(title.id);
    title.dispatchEvent(new Event('blur'));
    await fixture.whenStable();
    expect(root.textContent).toContain('A title, please.');
    expect(title.getAttribute('aria-invalid')).toBe('true');

    title.value = 'Plan';
    title.dispatchEvent(new Event('input'));
    await fixture.whenStable();
    expect(fixture.componentInstance.model().title).toBe('Plan');
    expect(root.textContent).not.toContain('A title, please.');
  });

  it('works outside a form field too', async () => {
    @Component({
      imports: [GlzInputDirective],
      changeDetection: ChangeDetectionStrategy.OnPush,
      template: `<input glzInput aria-label="Search" data-testid="search" />`,
    })
    class BareComponent {}
    const fixture = TestBed.createComponent(BareComponent);
    await fixture.whenStable();
    const input = element<HTMLInputElement>(fixture.nativeElement, 'search');
    expect(input.hasAttribute('aria-describedby')).toBe(false);
    expect(input.id).toBe('');
  });
});
