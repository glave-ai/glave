import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { type GlzDecision, GlzApprovalCardComponent } from './approval-card.component';

@Component({
  imports: [GlzApprovalCardComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-approval-card
      testId="approval"
      tool="bash"
      summary="Delete the build folder"
      [decision]="decision()"
      [busy]="busy()"
      (decide)="decisions.push($event)"
    >
      <pre>rm -rf build/</pre>
    </glz-approval-card>
  `,
})
class HostComponent {
  readonly decision = signal<GlzDecision | null>(null);
  readonly busy = signal(false);
  readonly decisions: GlzDecision[] = [];
}

function button(root: HTMLElement, testId: string): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);
}

describe('GlzApprovalCardComponent', () => {
  it('offers the three answers and emits the one chosen', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const card = root.querySelector('glz-approval-card');
    expect(card?.getAttribute('role')).toBe('group');
    expect(document.getElementById(card?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      'Approval needed',
    );
    expect(root.textContent).toContain('rm -rf build/');
    await expectAccessible(root);
    button(root, 'approval-allow-once')?.click();
    button(root, 'approval-allow-thread')?.click();
    button(root, 'approval-deny')?.click();
    expect(fixture.componentInstance.decisions).toEqual(['allow_once', 'allow_for_thread', 'deny']);
  });

  it('waits while busy and says what was decided in place', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.busy.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(button(root, 'approval-allow-once')?.disabled).toBe(true);
    fixture.componentInstance.decision.set('allow_for_thread');
    await fixture.whenStable();
    expect(button(root, 'approval-allow-once')).toBeNull();
    expect(root.querySelector('[data-testid="approval-decided"]')?.textContent).toBe(
      'Allowed for this thread',
    );
  });
});
