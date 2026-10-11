import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzConfirmDialogComponent } from './confirm-dialog.component';

@Component({
  imports: [GlzConfirmDialogComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-confirm-dialog
      testId="confirm"
      heading="Delete the thread?"
      message="It cannot be undone."
      confirmLabel="Delete"
      danger
      [(open)]="open"
      (decided)="decisions.push($event)"
    />
  `,
})
class HostComponent {
  readonly open = signal(true);
  readonly decisions: boolean[] = [];
}

function find(root: HTMLElement, testId: string): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);
}

describe('GlzConfirmDialogComponent', () => {
  it('asks, and says yes only when the person goes ahead', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('.message')?.textContent).toBe('It cannot be undone.');
    expect(find(root, 'confirm-ok')?.classList).toContain('glz-button--danger');
    await expectAccessible(root);
    find(root, 'confirm-ok')?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.decisions).toEqual([true]);
    expect(fixture.componentInstance.open()).toBe(false);
  });

  it('takes Cancel, Escape or the close button as no', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    find(root, 'confirm-cancel')?.click();
    await fixture.whenStable();
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    find(root, 'confirm-close')?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.decisions).toEqual([false, false]);
  });
});
