import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzReasoningComponent } from './reasoning.component';

@Component({
  imports: [GlzReasoningComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-reasoning testId="reasoning" [streaming]="streaming()" [(open)]="open"
    >Weighing it up</glz-reasoning
  >`,
})
class HostComponent {
  readonly streaming = signal(true);
  readonly open = signal(false);
}

describe('GlzReasoningComponent', () => {
  it('starts collapsed, says when thinking, and tracks being opened', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const details = root.querySelector('details');
    const summary = root.querySelector('[data-testid="reasoning-summary"]');
    expect(details?.open).toBe(false);
    expect(summary?.textContent).toBe('Thinking…');
    expect(root.querySelector('[data-testid="reasoning-text"]')?.textContent).toBe(
      'Weighing it up',
    );
    fixture.componentInstance.streaming.set(false);
    await fixture.whenStable();
    expect(summary?.textContent).toBe('Reasoning');
    if (details) {
      details.open = true;
      details.dispatchEvent(new Event('toggle'));
    }
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(true);
    await expectAccessible(root);
  });
});
