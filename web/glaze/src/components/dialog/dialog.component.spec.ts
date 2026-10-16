import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzDialogComponent } from './dialog.component';

@Component({
  imports: [GlzDialogComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-dialog testId="dialog" heading="Settings" [(open)]="open">
      <p>Body</p>
      <div glzDialogActions><button type="button">Save</button></div>
    </glz-dialog>
  `,
})
class HostComponent {
  readonly open = signal(false);
}

function dialogOf(root: HTMLElement): HTMLDialogElement {
  const found = root.querySelector<HTMLDialogElement>('[data-testid="dialog"]');
  if (found === null) {
    throw new Error('no dialog');
  }
  return found;
}

describe('GlzDialogComponent', () => {
  it('opens modally when open, named by its heading', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const dialog = dialogOf(root);
    expect(dialog.open).toBe(false);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    expect(dialog.open).toBe(true);
    expect(dialog.dataset['modal']).toBe('true');
    expect(dialog.getAttribute('aria-labelledby')).toBe(root.querySelector('h2')?.id);
    expect(root.querySelector('.content')?.textContent).toBe('Body');
    expect(root.querySelector('.actions button')?.textContent).toBe('Save');
    await expectAccessible(root);
  });

  it('closes from its button, the backdrop or the browser, keeping open in step', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    root.querySelector<HTMLButtonElement>('[data-testid="dialog-close"]')?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(false);
    expect(dialogOf(root).open).toBe(false);

    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    dialogOf(root).click();
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(false);

    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    dialogOf(root).close();
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(false);
  });

  it('keeps Escape to itself, so a drawer it was opened from stays open', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const heard: string[] = [];
    root.addEventListener('keydown', (event) => heard.push(event.key));
    const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true });
    root.querySelector('p')?.dispatchEvent(escape);
    root
      .querySelector('p')
      ?.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }));
    expect(heard).toEqual(['a']);
    expect(escape.defaultPrevented).toBe(false);
  });
});
