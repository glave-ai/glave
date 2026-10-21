import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzBadgeComponent } from './badge.component';

@Component({
  imports: [GlzBadgeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-badge tone="warning" data-testid="badge">2 approvals</glz-badge>
    <glz-badge data-testid="neutral">project</glz-badge>
  `,
})
class HostComponent {}

function badge(root: HTMLElement, testId: string): HTMLElement | null {
  return root.querySelector(`[data-testid="${testId}"]`);
}

describe('GlzBadgeComponent', () => {
  it('shows its text in its tone', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(badge(root, 'badge')?.textContent).toBe('2 approvals');
    expect(badge(root, 'badge')?.classList).toContain('glz-tone-warning');
    expect(badge(root, 'neutral')?.classList).toContain('glz-tone-neutral');
    await expectAccessible(root);
  });
});
