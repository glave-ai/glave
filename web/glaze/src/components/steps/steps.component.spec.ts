import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzReceiptComponent } from '../receipt/receipt.component';
import { type GlzRunStatus, GlzStepsComponent } from './steps.component';

@Component({
  imports: [GlzReceiptComponent, GlzStepsComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-steps testId="steps" steps="2 steps" [status]="status()" [facts]="facts" [(open)]="open">
      <glz-receipt summary="bash: cargo test" />
      <glz-receipt summary="edit src/parse.rs" />
    </glz-steps>
  `,
})
class HostComponent {
  readonly open = signal(false);
  readonly status = signal<GlzRunStatus>('done');
  readonly facts = ['4m 02s', '$0.04'];
}

function words(root: HTMLElement): readonly string[] {
  return Array.from(
    root.querySelectorAll('[data-testid="steps-summary"] > span:not(.glz-visually-hidden)'),
  ).map((each) => each.textContent.trim());
}

describe('GlzStepsComponent', () => {
  it('says what the run came to in one line, a disclosure that opens to its steps', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const summary = root.querySelector<HTMLElement>('[data-testid="steps-summary"]');
    expect(summary?.tagName).toBe('SUMMARY');
    // Done shows a check, read as a word only by screen readers.
    expect(summary?.querySelector('.state')?.textContent).toBe('Done');
    expect(summary?.querySelector('.state')?.classList).toContain('glz-visually-hidden');
    expect(words(root)).toEqual(['4m 02s', '$0.04', '2 steps']);
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

  it('says in words when the run failed, stopped, or waits', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    const root: HTMLElement = fixture.nativeElement;
    for (const [status, word] of [
      ['failed', 'failed'],
      ['stopped', 'stopped'],
      ['running', 'working'],
      ['waiting', 'waiting'],
    ] as const) {
      fixture.componentInstance.status.set(status);
      await fixture.whenStable();
      expect(words(root)[0]).toBe(word);
      expect(root.querySelector('glz-steps')?.classList).toContain(`status-${status}`);
      // Only a running run's ring turns.
      const turning = root.querySelector('.icon')?.classList.contains('glz-spin');
      expect(turning).toBe(status === 'running');
    }
  });

  it('slides its steps shut before it closes, and closes at once when nothing moves', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const details = root.querySelector('details');
    const fold = root.querySelector<HTMLElement>('[data-testid="steps-fold"]');
    const summary = root.querySelector<HTMLElement>('[data-testid="steps-summary"]');
    let slid: () => void = () => undefined;
    const sliding = new Promise<void>((resolve) => {
      slid = resolve;
    });
    const spy = vi
      .spyOn(fold!, 'getAnimations')
      .mockReturnValueOnce([{ finished: sliding } as unknown as Animation]);
    summary?.click();
    // Still open, the fold sliding shut; clicked again meanwhile, it slides on.
    expect(details?.open).toBe(true);
    expect(details?.classList).toContain('shutting');
    summary?.click();
    expect(details?.open).toBe(true);
    slid();
    await sliding;
    await fixture.whenStable();
    expect(details?.open).toBe(false);
    expect(details?.classList).not.toContain('shutting');
    details?.dispatchEvent(new Event('toggle'));
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(false);
    // Under reduced motion nothing slides: it closes as it is clicked.
    spy.mockRestore();
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    summary?.click();
    expect(details?.open).toBe(false);
  });

  it('opens when its parent opens it', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).querySelector('details')?.open).toBe(true);
  });
});
