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

@Component({
  imports: [GlzRadioGroupComponent, GlzRadioComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-radio-group label="Start from" cards [(value)]="start">
      <glz-radio value="folder" hint="Work on files you already have">A folder</glz-radio>
      <glz-radio value="empty">Empty</glz-radio>
    </glz-radio-group>
  `,
})
class CardsHostComponent {
  readonly start = signal<string | null>('folder');
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

  it('names a radio by its label and describes it by its hint, drawn as a card in a group of cards', async () => {
    const fixture = TestBed.createComponent(CardsHostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const [folder, empty] = Array.from(root.querySelectorAll<HTMLInputElement>('input'));
    const named = (id: string | null | undefined): string | undefined =>
      root.querySelector(`[id="${id ?? ''}"]`)?.textContent.trim();
    expect(named(folder?.getAttribute('aria-labelledby'))).toBe('A folder');
    expect(named(folder?.getAttribute('aria-describedby'))).toBe('Work on files you already have');
    // Without a hint, the label names it as before.
    expect(empty?.hasAttribute('aria-labelledby')).toBe(false);
    expect(empty?.hasAttribute('aria-describedby')).toBe(false);
    expect(root.querySelectorAll('label.card')).toHaveLength(2);
    await expectAccessible(root);
    empty?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.start()).toBe('empty');
  });

  it('is no card outside a group of cards', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).querySelector('label.card')).toBeNull();
  });
});
