import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzShortcutDirective } from './shortcut.directive';

@Component({
  imports: [GlzShortcutDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button type="button" glzShortcut="ctrl+k" (click)="clicks.set(clicks() + 1)">Search</button>
    <button type="button" glzShortcut="/" (pressed)="slashes.set(slashes() + 1)">Find</button>
    <input aria-label="Draft" data-testid="draft" />
  `,
})
class HostComponent {
  readonly clicks = signal(0);
  readonly slashes = signal(0);
}

describe('GlzShortcutDirective', () => {
  it('clicks its element on the shortcut, but not while typing a plain key', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    // jsdom is not an Apple platform, so `mod` would be Ctrl; plain names work everywhere.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
    expect(fixture.componentInstance.slashes()).toBe(1);
    const draft = (fixture.nativeElement as HTMLElement).querySelector('[data-testid="draft"]');
    draft?.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
    expect(fixture.componentInstance.slashes()).toBe(1);
    fixture.destroy();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
    expect(fixture.componentInstance.slashes()).toBe(1);
  });
});
