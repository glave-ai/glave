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
      summary="Run this command?"
      detail="rm -rf build/"
      [caution]="caution()"
      [reason]="reason()"
      [onlyOnce]="onlyOnce()"
      [decision]="decision()"
      [busy]="busy()"
      (decide)="decisions.push($event)"
    />
  `,
})
class HostComponent {
  readonly caution = signal('');
  readonly reason = signal('');
  readonly onlyOnce = signal(false);
  readonly decision = signal<GlzDecision | null>(null);
  readonly busy = signal(false);
  readonly decisions: GlzDecision[] = [];
}

function button(root: HTMLElement, testId: string): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);
}

describe('GlzApprovalCardComponent', () => {
  it('needs the person: says what it asks, and offers the three answers, the primary first', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const card = root.querySelector('glz-approval-card');
    expect(card?.getAttribute('role')).toBe('group');
    const names = (card?.getAttribute('aria-labelledby') ?? '').split(' ');
    const text = (id: string): string =>
      document.getElementById(id)?.textContent.replace(/\s+/gu, ' ').trim() ?? '';
    expect(names.map(text)).toEqual(['Needs you', 'Run this command? rm -rf build/']);
    expect(root.querySelector('.detail')?.textContent).toBe('rm -rf build/');
    const buttons = Array.from(root.querySelectorAll('.actions button'));
    expect(buttons.map((each) => each.textContent.trim())).toEqual([
      'Allow once',
      'Allow for this thread',
      'Deny',
    ]);
    expect(buttons[0]?.classList).toContain('glz-button--primary');
    await expectAccessible(root);
    button(root, 'approval-allow-once')?.click();
    button(root, 'approval-allow-thread')?.click();
    button(root, 'approval-deny')?.click();
    expect(fixture.componentInstance.decisions).toEqual(['allow_once', 'allow_for_thread', 'deny']);
    // Only the mode asks: nothing more to say, and nothing describes the card.
    expect(root.querySelector('[data-testid="approval-caution"]')).toBeNull();
    expect(root.querySelector('[data-testid="approval-reason"]')).toBeNull();
    expect(card?.hasAttribute('aria-describedby')).toBe(false);
  });

  it('quotes the agent’s reason as its words, apart from the caution, and asks only once', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.caution.set('It runs outside the sandbox.');
    fixture.componentInstance.reason.set('cargo add writes to ~/.cargo/registry.');
    fixture.componentInstance.onlyOnce.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const caution = root.querySelector('[data-testid="approval-caution"]');
    const reason = root.querySelector('[data-testid="approval-reason"]');
    expect(reason?.textContent.replace(/\s+/gu, ' ').trim()).toBe(
      'The model says cargo add writes to ~/.cargo/registry.',
    );
    expect(reason?.querySelector('q')?.textContent).toBe('cargo add writes to ~/.cargo/registry.');
    expect(caution?.contains(reason ?? null)).toBe(false);
    expect(reason?.classList).not.toContain('caution');
    const card = root.querySelector('glz-approval-card');
    expect(card?.getAttribute('aria-describedby')).toBe(`${caution?.id ?? ''} ${reason?.id ?? ''}`);
    const buttons = Array.from(root.querySelectorAll('.actions button'));
    expect(buttons.map((each) => each.textContent.trim())).toEqual(['Allow once', 'Deny']);
    expect(button(root, 'approval-allow-thread')).toBeNull();
    await expectAccessible(root);
  });

  it('says why it asks in a sentence of its own, apart from what would run', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.caution.set('It deletes build/ for good.');
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const caution = root.querySelector('[data-testid="approval-caution"]');
    expect(caution?.textContent).toBe('It deletes build/ for good.');
    expect(root.querySelector('.detail')?.textContent).toBe('rm -rf build/');
    expect(root.querySelector('.summary')?.contains(caution)).toBe(false);
    const card = root.querySelector('glz-approval-card');
    expect(card?.getAttribute('aria-describedby')).toBe(caution?.id);
    await expectAccessible(root);
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
    // Answered, it no longer needs the person: no label, and it rests.
    const card = root.querySelector('glz-approval-card');
    expect(root.querySelector('.label')).toBeNull();
    expect(card?.classList).toContain('glz-approval-card--decided');
    expect(
      document.getElementById(card?.getAttribute('aria-labelledby') ?? '')?.textContent.trim(),
    ).toMatch(/^Run this command\?/u);
  });
});
