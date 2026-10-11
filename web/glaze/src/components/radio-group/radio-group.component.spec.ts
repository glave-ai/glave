import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule } from '@angular/forms';
import { FormField, form } from '@angular/forms/signals';

import { expectAccessible } from '../../testing/axe';
import { GlzRadioComponent } from '../radio/radio.component';
import { GlzRadioGroupComponent } from './radio-group.component';

@Component({
  imports: [GlzRadioGroupComponent, GlzRadioComponent, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-radio-group label="Approval" [formControl]="mode">
      <glz-radio value="ask">Ask</glz-radio>
      <glz-radio value="auto">Auto</glz-radio>
    </glz-radio-group>
  `,
})
class ReactiveHostComponent {
  readonly mode = new FormControl<string | null>('ask');
}

@Component({
  imports: [GlzRadioGroupComponent, GlzRadioComponent, FormField],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-radio-group label="Size" [formField]="choice.size">
      <glz-radio value="s">Small</glz-radio>
      <glz-radio value="l">Large</glz-radio>
    </glz-radio-group>
  `,
})
class SignalHostComponent {
  readonly model = signal({ size: 'l' });
  readonly choice = form(this.model);
}

@Component({
  imports: [GlzRadioGroupComponent, GlzRadioComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-radio-group label="Tone" [(value)]="tone">
      <glz-radio value="warm">Warm</glz-radio>
      <glz-radio value="cool" disabled>Cool</glz-radio>
    </glz-radio-group>
  `,
})
class ModelHostComponent {
  readonly tone = signal<string | null>(null);
}

function radios(root: HTMLElement): readonly HTMLInputElement[] {
  return Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
}

describe('GlzRadioGroupComponent', () => {
  it('binds to reactive forms, both ways, and can be disabled by the form', async () => {
    const fixture = TestBed.createComponent(ReactiveHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const [ask, auto] = radios(root);
    expect(ask?.checked).toBe(true);
    expect(ask?.name).toBe(auto?.name);
    expect(root.querySelector('[role="radiogroup"]')?.getAttribute('aria-label')).toBe('Approval');
    auto?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.mode.value).toBe('auto');
    fixture.componentInstance.mode.setValue('ask');
    await fixture.whenStable();
    expect(ask?.checked).toBe(true);
    fixture.componentInstance.mode.disable();
    await fixture.whenStable();
    expect(auto?.disabled).toBe(true);
    await expectAccessible(root);
  });

  it('binds to signal forms', async () => {
    const fixture = TestBed.createComponent(SignalHostComponent);
    await fixture.whenStable();
    const [small, large] = radios(fixture.nativeElement);
    expect(large?.checked).toBe(true);
    small?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.model().size).toBe('s');
  });

  it('binds two ways with a model, and keeps a disabled choice out of reach', async () => {
    const fixture = TestBed.createComponent(ModelHostComponent);
    await fixture.whenStable();
    const [warm, cool] = radios(fixture.nativeElement);
    expect(cool?.disabled).toBe(true);
    warm?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.tone()).toBe('warm');
  });
});
