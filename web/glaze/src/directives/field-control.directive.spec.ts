import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';

import { GlzFormFieldComponent } from '../components/form-field/form-field.component';
import { GlzFieldControlDirective } from './field-control.directive';

@Component({
  imports: [GlzFormFieldComponent, GlzFieldControlDirective, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Colour" hint="For the space's badge">
      <input type="color" glzFieldControl data-testid="colour" [formControl]="colour" />
    </glz-form-field>
  `,
})
class HostComponent {
  readonly colour = new FormControl('', { validators: [Validators.required] });
}

describe('GlzFieldControlDirective', () => {
  it('links any native control to its field', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const colour = root.querySelector('[data-testid="colour"]');
    expect(root.querySelector('label')?.htmlFor).toBe(colour?.id);
    expect(colour?.getAttribute('aria-describedby')).toBe(root.querySelector('.hint')?.id);
    expect(colour?.getAttribute('aria-required')).toBe('true');
  });
});
