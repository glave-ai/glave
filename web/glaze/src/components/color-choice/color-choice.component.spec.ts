import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzColorChoiceComponent, type GlzColorOption } from './color-choice.component';

@Component({
  imports: [GlzColorChoiceComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-color-choice
      testId="accent"
      label="Accent"
      anyLabel="Pick your own color"
      [options]="options"
      [(value)]="accent"
    />
  `,
})
class HostComponent {
  readonly options: readonly GlzColorOption[] = [
    { value: 'plain', label: 'No accent' },
    { value: '#0f766e', label: 'Teal', color: '#0f766e' },
  ];
  readonly accent = signal<string | null>('plain');
}

function radios(root: HTMLElement): readonly HTMLInputElement[] {
  return Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
}

function picker(root: HTMLElement): HTMLInputElement | null {
  return root.querySelector<HTMLInputElement>('[data-testid="accent-any"]');
}

describe('GlzColorChoiceComponent', () => {
  it('offers words and named swatches as one group of native radios', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[role="radiogroup"]')?.getAttribute('aria-label')).toBe('Accent');
    expect(root.textContent).toContain('No accent');
    expect(radios(root).map((radio) => radio.checked)).toEqual([true, false]);
    expect(radios(root)[1]?.getAttribute('aria-label')).toBe('Teal');
    expect(radios(root)[1]?.closest<HTMLElement>('.swatch')?.style.backgroundColor).toBe(
      'rgb(15, 118, 110)',
    );
    expect(picker(root)?.getAttribute('aria-label')).toBe('Pick your own color');
    await expectAccessible(root);
  });

  it('chooses a swatch, or any exact color through the browser’s picker', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    radios(root)[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.accent()).toBe('#0f766e');
    expect(root.querySelector('.any')?.classList).not.toContain('picked');

    const any = picker(root);
    if (any !== null) {
      any.value = '#123456';
      any.dispatchEvent(new Event('change'));
    }
    await fixture.whenStable();
    expect(fixture.componentInstance.accent()).toBe('#123456');
    expect(root.querySelector('.any')?.classList).toContain('picked');
    expect(radios(root).map((radio) => radio.checked)).toEqual([false, false]);
    expect(any?.value).toBe('#123456');
  });
});
