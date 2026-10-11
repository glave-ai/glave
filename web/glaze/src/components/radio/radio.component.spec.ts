import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzRadioGroupComponent } from '../radio-group/radio-group.component';
import { GlzRadioComponent } from './radio.component';

@Component({
  imports: [GlzRadioGroupComponent, GlzRadioComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-radio-group label="Answer" [(value)]="answer" (touch)="touched.set(true)">
      <glz-radio value="yes">Yes</glz-radio>
    </glz-radio-group>
  `,
})
class HostComponent {
  readonly answer = signal<string | null>(null);
  readonly touched = signal(false);
}

describe('GlzRadioComponent', () => {
  it('is a labelled native radio that picks its value and marks the group touched', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const radio = root.querySelector<HTMLInputElement>('input[type="radio"]');
    expect(radio?.closest('label')?.textContent).toContain('Yes');
    radio?.click();
    radio?.dispatchEvent(new Event('blur'));
    await fixture.whenStable();
    expect(fixture.componentInstance.answer()).toBe('yes');
    expect(fixture.componentInstance.touched()).toBe(true);
    expect(radio?.checked).toBe(true);
    await expectAccessible(root);
  });
});
