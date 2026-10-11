import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule } from '@angular/forms';

import { GlzFormFieldComponent } from '../components/form-field/form-field.component';
import { expectAccessible } from '../testing/axe';
import { GlzSliderDirective } from './slider.directive';

@Component({
  imports: [GlzFormFieldComponent, GlzSliderDirective, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-form-field label="Budget">
      <input
        type="range"
        glzSlider
        data-testid="budget"
        min="0"
        max="100"
        valueText="Half"
        [formControl]="budget"
      />
    </glz-form-field>
  `,
})
class HostComponent {
  readonly budget = new FormControl(50);
}

describe('GlzSliderDirective', () => {
  it('is a labelled native range bound to the form, with a value text', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const slider = root.querySelector<HTMLInputElement>('[data-testid="budget"]');
    expect(slider?.classList).toContain('glz-slider');
    expect(root.querySelector('label')?.htmlFor).toBe(slider?.id);
    expect(slider?.getAttribute('aria-valuetext')).toBe('Half');
    expect(slider?.value).toBe('50');
    if (slider) {
      slider.value = '70';
      slider.dispatchEvent(new Event('input'));
    }
    expect(fixture.componentInstance.budget.value).toBe(70);
    await expectAccessible(root);
  });
});
