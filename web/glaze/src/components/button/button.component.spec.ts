import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzButtonComponent, buttonClasses } from './button.component';

@Component({
  imports: [GlzButtonComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button glzButton variant="primary" data-testid="save" (click)="save()">Save</button>
    <a glzButton href="/help" size="sm" data-testid="help">Help</a>
  `,
})
class HostComponent {
  readonly saved = signal(0);
  save(): void {
    this.saved.update((count) => count + 1);
  }
}

describe('GlzButtonComponent', () => {
  it('keeps the native element and its behaviour, styled by variant and size', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const save = root.querySelector<HTMLButtonElement>('[data-testid="save"]');
    expect(save?.classList).toContain('glz-button--primary');
    save?.click();
    expect(fixture.componentInstance.saved()).toBe(1);
    expect(root.querySelector('[data-testid="help"]')?.classList).toContain('glz-button--sm');
    await expectAccessible(root);
  });

  it('names its classes from variant and size', () => {
    expect(buttonClasses('ghost', 'lg')).toBe('glz-button glz-button--ghost glz-button--lg');
  });
});
