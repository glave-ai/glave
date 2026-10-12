import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzReceiptComponent } from '../receipt/receipt.component';
import { GlzStepsComponent } from './steps.component';

@Component({
  imports: [GlzReceiptComponent, GlzStepsComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-steps testId="steps" summary="Ran 2 steps" [(open)]="open">
      <glz-receipt tool="bash" summary="cargo test" />
      <glz-receipt tool="edit" summary="src/parse.rs" />
    </glz-steps>
  `,
})
class HostComponent {
  readonly open = signal(false);
}

describe('GlzStepsComponent', () => {
  it('folds its steps under one line, a disclosure that opens', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const summary = root.querySelector<HTMLElement>('[data-testid="steps-summary"]');
    expect(summary?.tagName).toBe('SUMMARY');
    expect(summary?.textContent.trim()).toBe('Ran 2 steps');
    const details = root.querySelector('details');
    expect(details?.open).toBe(false);
    expect(root.querySelectorAll('[data-testid="steps-steps"] glz-receipt')).toHaveLength(2);
    await expectAccessible(root);

    summary?.click();
    details?.dispatchEvent(new Event('toggle'));
    await fixture.whenStable();
    expect(details?.open).toBe(true);
    expect(fixture.componentInstance.open()).toBe(true);
    await expectAccessible(root);
  });

  it('opens when its parent opens it', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).querySelector('details')?.open).toBe(true);
  });
});
