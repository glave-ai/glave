import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';

import { GlzInputDirective } from '../../directives/input.directive';
import { expectAccessible } from '../../testing/axe';
import { GlzRadioGroupComponent } from '../radio-group/radio-group.component';
import { GlzRadioComponent } from '../radio/radio.component';
import { GlzFormFieldComponent } from './form-field.component';

@Component({
  imports: [GlzFormFieldComponent, GlzInputDirective, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Email" hint="We never share it">
      <input glzInput type="email" [formControl]="email" />
    </glz-form-field>
  `,
})
class InputHostComponent {
  readonly email = new FormControl('', { validators: [Validators.required] });
}

@Component({
  imports: [GlzFormFieldComponent, GlzRadioGroupComponent, GlzRadioComponent, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Size">
      <glz-radio-group [formControl]="size">
        <glz-radio value="s">Small</glz-radio>
        <glz-radio value="l">Large</glz-radio>
      </glz-radio-group>
    </glz-form-field>
  `,
})
class GroupHostComponent {
  readonly size = new FormControl('s');
}

describe('GlzFormFieldComponent', () => {
  it('labels a native control, marks it required and shows the hint', async () => {
    const fixture = TestBed.createComponent(InputHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const label = root.querySelector('label');
    expect(label?.textContent).toContain('Email');
    expect(label?.querySelector('.required')?.getAttribute('aria-hidden')).toBe('true');
    expect(root.querySelector('.hint')?.textContent).toContain('We never share it');
    await expectAccessible(root);
  });

  it('names a group with a span its control points at', async () => {
    const fixture = TestBed.createComponent(GroupHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('label.label')).toBeNull();
    const name = root.querySelector('span.label');
    expect(root.querySelector('[role="radiogroup"]')?.getAttribute('aria-labelledby')).toBe(
      name?.id,
    );
    await expectAccessible(root);
  });
});
