import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseOverlaysComponent } from './showcase-overlays.component';

/** The showcase's dialogs, rendered and settled. */
async function overlays(): Promise<ComponentFixture<ShowcaseOverlaysComponent>> {
  const fixture = TestBed.createComponent(ShowcaseOverlaysComponent);
  await fixture.whenStable();
  return fixture;
}

/** Clicks the element `testId` names, then waits until the page has settled. */
async function click(
  fixture: ComponentFixture<ShowcaseOverlaysComponent>,
  testId: string,
): Promise<void> {
  const root: HTMLElement = fixture.nativeElement;
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
  await fixture.whenStable();
}

/** The open dialog, if one is. */
function open(fixture: ComponentFixture<ShowcaseOverlaysComponent>): HTMLDialogElement | null {
  const root: HTMLElement = fixture.nativeElement;
  return root.querySelector('dialog[open]');
}

// One test per dialog: each settles on its own state, and stays well inside a test's time on
// a busy machine.
describe('ShowcaseOverlaysComponent', () => {
  it('opens the dialog and saves', async () => {
    const fixture = await overlays();
    await click(fixture, 'showcase-open-dialog');
    expect(open(fixture)?.textContent).toContain('Rename the thread');
    await expectAccessible(fixture.nativeElement as HTMLElement);
    await click(fixture, 'showcase-dialog-save');
    expect(open(fixture)).toBeNull();
  });

  it('confirms a deletion', async () => {
    const fixture = await overlays();
    await click(fixture, 'showcase-open-confirm');
    expect(open(fixture)?.textContent).toContain('Delete the thread?');
    await click(fixture, 'showcase-confirm-ok');
    expect(open(fixture)).toBeNull();
  });

  it('chooses in a long dialog', async () => {
    const fixture = await overlays();
    await click(fixture, 'showcase-open-long');
    expect(open(fixture)?.querySelectorAll('glz-checkbox')).toHaveLength(24);
    await expectAccessible(fixture.nativeElement as HTMLElement);
    await click(fixture, 'showcase-long-done');
    expect(open(fixture)).toBeNull();
  });
});
