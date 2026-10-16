import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzBadgeComponent } from './badge.component';

@Component({
  imports: [GlzBadgeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-badge tone="warning" data-testid="badge">2 approvals</glz-badge>
    <glz-badge tone="accent" variant="dot" data-testid="dot">needs you</glz-badge>
    <glz-badge variant="plain" data-testid="plain">running</glz-badge>
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
    expect(badge(root, 'badge')?.classList).toContain('glz-badge--pill');
    await expectAccessible(root);
  });

  it('as a dot, keeps its text for screen readers only; plain, shows it as is', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const dot = badge(root, 'dot');
    expect(dot?.classList).toContain('glz-badge--dot');
    expect(dot?.textContent).toBe('needs you');
    expect(dot?.querySelector('.glz-visually-hidden')?.textContent).toBe('needs you');
    expect(badge(root, 'plain')?.classList).toContain('glz-badge--plain');
    expect(badge(root, 'plain')?.querySelector('.glz-visually-hidden')).toBeNull();
    await expectAccessible(root);
  });
});
