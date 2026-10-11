import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { GlzMoney } from '../../lib/format';
import { expectAccessible } from '../../testing/axe';
import { type GlzReceiptStatus, GlzReceiptComponent } from './receipt.component';

@Component({
  imports: [GlzReceiptComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-receipt
      tool="bash"
      summary="cargo test"
      [status]="status()"
      [durationMs]="4200"
      [tokens]="1200"
      [cost]="cost"
      [(open)]="open"
    >
      <pre>test result: ok</pre>
    </glz-receipt>
  `,
})
class HostComponent {
  readonly status = signal<GlzReceiptStatus>('running');
  readonly open = signal(false);
  readonly cost: GlzMoney = { micros: 4200, currency: 'USD' };
}

describe('GlzReceiptComponent', () => {
  it('shows what ran, its status, time, tokens and cost on one line', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const line = root.querySelector('[data-testid="receipt-summary"]');
    expect(line?.querySelector('.tool')?.textContent).toBe('bash');
    expect(line?.querySelector('.what')?.textContent).toBe('cargo test');
    expect(line?.querySelector('.meta')?.textContent).toMatch(/^4 s · 1\.2K tokens · /u);
    expect(line?.querySelector('glz-icon')?.getAttribute('aria-label')).toBe('Running');
    fixture.componentInstance.status.set('failed');
    await fixture.whenStable();
    expect(line?.querySelector('glz-icon')?.getAttribute('aria-label')).toBe('Failed');
    expect(root.querySelector('glz-receipt')?.classList).toContain('status-failed');
    await expectAccessible(root);
  });

  it('opens to its evidence', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const details = (fixture.nativeElement as HTMLElement).querySelector('details');
    expect(details?.querySelector('[data-testid="receipt-evidence"]')?.textContent).toContain(
      'test result: ok',
    );
    if (details) {
      details.open = true;
      details.dispatchEvent(new Event('toggle'));
    }
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(true);
  });
});
