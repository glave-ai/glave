import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { X } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from './icon-button.component';

@Component({
  imports: [GlzIconButtonComponent, GlzIconComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button glzIconButton data-testid="close" label="Close" [tooltip]="tooltip()">
      <glz-icon [icon]="x" />
    </button>
  `,
})
class HostComponent {
  protected readonly x = X;
  readonly tooltip = signal(true);
}

describe('GlzIconButtonComponent', () => {
  it('is named by its label, shown on hover unless turned off', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const close = (fixture.nativeElement as HTMLElement).querySelector('[data-testid="close"]');
    expect(close?.getAttribute('aria-label')).toBe('Close');
    expect(close?.getAttribute('title')).toBe('Close');
    expect(close?.classList).toContain('glz-button--ghost');
    await expectAccessible(fixture.nativeElement);

    fixture.componentInstance.tooltip.set(false);
    await fixture.whenStable();
    expect(close?.hasAttribute('title')).toBe(false);
  });
});
