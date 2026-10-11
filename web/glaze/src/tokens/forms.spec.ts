import { ChangeDetectionStrategy, Component, Directive } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import {
  type ControlValueAccessor,
  FormControl,
  NgControl,
  ReactiveFormsModule,
  Validators,
} from '@angular/forms';

import { bindValueAccessor, injectFormState } from './forms';

/** A bare control that reads its form state and is its own value accessor. */
@Directive({ selector: '[appProbe]', exportAs: 'probe' })
class ProbeDirective implements ControlValueAccessor {
  readonly state = injectFormState();
  written: unknown = null;
  constructor() {
    bindValueAccessor(this);
  }
  writeValue(value: unknown): void {
    this.written = value;
  }
  registerOnChange(): void {
    // The probe never changes the value itself.
  }
  registerOnTouched(): void {
    // Nor is it touched.
  }
}

@Component({
  imports: [ProbeDirective, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <span appProbe #bound="probe" [formControl]="name"></span>
    <span appProbe #bare="probe"></span>
  `,
})
class HostComponent {
  readonly name = new FormControl('', { validators: [Validators.required] });
}

describe('forms', () => {
  it('binds a value accessor and reads reactive state, required included', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const probes = fixture.debugElement
      .queryAll((node) => node.injector.get(ProbeDirective, null) !== null)
      .map((node) => node.injector.get(ProbeDirective));
    const [bound, bare] = probes;
    expect(bound?.written).toBe('');
    expect(bound?.state.required()).toBe(true);
    expect(bound?.state.invalid()).toBe(true);
    expect(bound?.state.touched()).toBe(false);
    fixture.componentInstance.name.markAsTouched();
    fixture.componentInstance.name.setValue('Ada');
    expect(bound?.state.touched()).toBe(true);
    expect(bound?.state.invalid()).toBe(false);
    expect(bound?.written).toBe('Ada');
    expect(bare?.state.invalid()).toBe(false);
    expect(bare?.state.messages()).toEqual([]);
    expect(TestBed.inject(NgControl, null)).toBeNull();
  });
});
