import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../testing/axe';
import { GlzSplitterDirective } from './splitter.directive';

@Component({
  imports: [GlzSplitterDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <nav #files id="files" aria-label="Files">Files</nav>
    <div
      glzSplitter="files"
      side="start"
      aria-label="Resize the files"
      data-testid="edge"
      [glzSplitterPanel]="files"
      [minWidth]="200"
      [maxWidth]="400"
      [(open)]="open"
      [(width)]="width"
    ></div>
  `,
})
class HostComponent {
  readonly open = signal(true);
  readonly width = signal<number | null>(300);
}

function edgeOf(root: HTMLElement): HTMLElement {
  const edge = root.querySelector<HTMLElement>('[data-testid="edge"]');
  if (edge === null) {
    throw new Error('no edge');
  }
  Object.assign(edge, { setPointerCapture: () => undefined });
  return edge;
}

function pointer(edge: HTMLElement, type: string, clientX: number): void {
  const event = new MouseEvent(type, { clientX, button: 0, bubbles: true, cancelable: true });
  Object.assign(event, { pointerId: 7 });
  edge.dispatchEvent(event);
}

describe('GlzSplitterDirective', () => {
  it('is a focusable separator that says the width of the panel it controls', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const edge = edgeOf(fixture.nativeElement as HTMLElement);
    expect(edge.getAttribute('role')).toBe('separator');
    expect(edge.tabIndex).toBe(0);
    expect(edge.getAttribute('aria-orientation')).toBe('vertical');
    expect(edge.getAttribute('aria-controls')).toBe('files');
    expect(edge.getAttribute('aria-valuenow')).toBe('300');
    expect(edge.getAttribute('aria-valuemax')).toBe('400');
    await expectAccessible(fixture.nativeElement as HTMLElement);
  });

  it('widens a panel at the start as its edge moves right, and collapses it past half its minimum', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const host = fixture.componentInstance;
    const edge = edgeOf(fixture.nativeElement as HTMLElement);
    pointer(edge, 'pointerdown', 300);
    pointer(edge, 'pointermove', 301);
    expect(host.width()).toBe(300);
    pointer(edge, 'pointermove', 350);
    expect(host.width()).toBe(350);
    pointer(edge, 'pointermove', 100);
    expect(host.width()).toBe(200);
    expect(host.open()).toBe(true);
    pointer(edge, 'pointermove', 50);
    expect(host.open()).toBe(false);
    expect(host.width()).toBe(200);
    pointer(edge, 'pointercancel', 50);
    // From collapsed, a drag starts at nothing and opens at the minimum.
    pointer(edge, 'pointerdown', 0);
    pointer(edge, 'pointermove', 120);
    expect(host.open()).toBe(true);
    expect(host.width()).toBe(200);
    pointer(edge, 'pointerup', 120);
  });

  it('moves with the arrows, Home and End, and toggles with Enter', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const host = fixture.componentInstance;
    const edge = edgeOf(fixture.nativeElement as HTMLElement);
    const press = (key: string): void => {
      edge.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
    };
    press('ArrowRight');
    expect(host.width()).toBe(316);
    press('End');
    expect(host.width()).toBe(400);
    press('Home');
    expect(host.width()).toBe(200);
    press('ArrowLeft');
    expect(host.width()).toBe(200);
    press('Enter');
    expect(host.open()).toBe(false);
    press('ArrowLeft');
    expect(host.open()).toBe(false);
    press('End');
    expect(host.open()).toBe(true);
  });
});
