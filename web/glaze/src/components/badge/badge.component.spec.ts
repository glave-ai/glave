import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzBadgeComponent } from './badge.component';

@Component({
  imports: [GlzBadgeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-badge tone="warning" data-testid="badge">2 approvals</glz-badge>`,
})
class HostComponent {}

describe('GlzBadgeComponent', () => {
  it('shows its text in its tone', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const badge = (fixture.nativeElement as HTMLElement).querySelector('[data-testid="badge"]');
    expect(badge?.textContent).toBe('2 approvals');
    expect(badge?.classList).toContain('glz-tone-warning');
    await expectAccessible(fixture.nativeElement);
  });
});
