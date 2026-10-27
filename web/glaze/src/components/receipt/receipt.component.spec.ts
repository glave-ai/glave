import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { SquareTerminal } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { type GlzReceiptStatus, GlzReceiptComponent } from './receipt.component';

@Component({
  imports: [GlzReceiptComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-receipt
      testId="receipt"
      summary="bash: cargo test"
      [icon]="terminal"
      [status]="status()"
      [(open)]="open"
    >
      <pre>test result: ok</pre>
    </glz-receipt>
  `,
})
class HostComponent {
  readonly status = signal<GlzReceiptStatus>('running');
  readonly open = signal(false);
  readonly terminal = SquareTerminal;
}

describe('GlzReceiptComponent', () => {
  it('shows what ran and its status on one line', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const line = root.querySelector('[data-testid="receipt-summary"]');
    expect(line?.querySelector('.what')?.textContent).toBe('bash: cargo test');
    expect(line?.querySelector('.status')?.getAttribute('aria-label')).toBe('Running');
    // Its ring turns while it runs, as every ring does (still under reduced motion).
    expect(line?.querySelector('.status')?.classList).toContain('glz-spin');
    // The tool's icon goes first and is decorative: the line says what it is.
    expect(line?.firstElementChild?.classList).toContain('kind');
    expect(line?.querySelector('.kind')?.getAttribute('aria-hidden')).toBe('true');
    fixture.componentInstance.status.set('failed');
    await fixture.whenStable();
    expect(line?.querySelector('.status')?.getAttribute('aria-label')).toBe('Failed');
    expect(line?.querySelector('.status')?.classList).not.toContain('glz-spin');
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
