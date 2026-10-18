import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseOverlaysComponent } from './showcase-overlays.component';

function click(root: HTMLElement, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('ShowcaseOverlaysComponent', () => {
  it('opens the dialog, saves, confirms a deletion, and chooses in a long dialog', async () => {
    const fixture = TestBed.createComponent(ShowcaseOverlaysComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    click(root, 'showcase-open-dialog');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')?.textContent).toContain('Rename the thread');
    await expectAccessible(root);
    click(root, 'showcase-dialog-save');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')).toBeNull();

    click(root, 'showcase-open-confirm');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')?.textContent).toContain('Delete the thread?');
    click(root, 'showcase-confirm-ok');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')).toBeNull();

    click(root, 'showcase-open-long');
    await fixture.whenStable();
    expect(root.querySelectorAll('[data-testid="showcase-long-body"] glz-checkbox')).toHaveLength(
      24,
    );
    await expectAccessible(root);
    click(root, 'showcase-long-done');
    await fixture.whenStable();
    expect(root.querySelector('dialog[open]')).toBeNull();
  });
});
