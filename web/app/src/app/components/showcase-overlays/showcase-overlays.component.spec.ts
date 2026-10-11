import { TestBed } from '@angular/core/testing';
import { GlzToastService } from '@glave/glaze';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseOverlaysComponent } from './showcase-overlays.component';

function click(root: HTMLElement, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('ShowcaseOverlaysComponent', () => {
  it('opens the dialog, saves, and confirms a deletion', async () => {
    const fixture = TestBed.createComponent(ShowcaseOverlaysComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const toasts = TestBed.inject(GlzToastService);
    click(root, 'showcase-open-dialog');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')?.textContent).toContain('Rename the thread');
    await expectAccessible(root);
    click(root, 'showcase-dialog-save');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')).toBeNull();
    expect(toasts.toasts().at(-1)?.message).toBe('Renamed');

    click(root, 'showcase-open-confirm');
    await fixture.whenStable();
    click(root, 'confirm-ok');
    await fixture.whenStable();
    expect(toasts.toasts().at(-1)?.message).toBe('Thread deleted');
  });

  it('shows toasts', async () => {
    const fixture = TestBed.createComponent(ShowcaseOverlaysComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const toasts = TestBed.inject(GlzToastService);
    click(root, 'showcase-toast');
    expect(toasts.toasts().at(-1)?.message).toBe('The server restarted.');
    expect(root.querySelector('[data-testid="showcase-menu"]')?.getAttribute('aria-haspopup')).toBe(
      'menu',
    );
  });
});
