import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { ChangeDetectionStrategy, Component, signal, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { BehaviorSubject, type Observable } from 'rxjs';

import { expectAccessible } from '../../testing/axe';
import { GlzPanelLayoutComponent } from './panel-layout.component';

@Component({
  imports: [GlzPanelLayoutComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-panel-layout
      testId="tools"
      label="Tools"
      [empty]="empty()"
      [(open)]="open"
      [(width)]="width"
    >
      <main>Content</main>
      <section glzPanel aria-label="Files"><a href="/a">A file</a></section>
    </glz-panel-layout>
  `,
})
class HostComponent {
  readonly layout = viewChild.required(GlzPanelLayoutComponent);
  readonly open = signal(false);
  readonly width = signal<number | null>(null);
  readonly empty = signal(false);
}

class FakeBreakpoints {
  readonly matches = new BehaviorSubject<BreakpointState>({ matches: false, breakpoints: {} });
  observe(): Observable<BreakpointState> {
    return this.matches;
  }
}

function part(root: HTMLElement, name: string): HTMLElement {
  const found = root.querySelector<HTMLElement>(`[data-testid="tools-${name}"]`);
  if (found === null) {
    throw new Error(`no ${name}`);
  }
  return found;
}

function key(element: HTMLElement, name: string): void {
  element.dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true }));
}

describe('GlzPanelLayoutComponent', () => {
  let breakpoints: FakeBreakpoints;

  beforeEach(() => {
    breakpoints = new FakeBreakpoints();
    TestBed.configureTestingModule({
      providers: [{ provide: BreakpointObserver, useValue: breakpoints }],
    });
  });

  it('starts closed, and opens beside the content on wide screens', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(part(root, 'panel').inert).toBe(true);
    fixture.componentInstance.layout().toggle();
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(true);
    expect(part(root, 'panel').inert).toBe(false);
    expect(part(root, 'panel').getAttribute('aria-modal')).toBeNull();
    expect(root.querySelector('[data-testid="tools-scrim"]')).toBeNull();
    await expectAccessible(root);
  });

  it('resizes from the keyboard within its bounds, and says its width', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    fixture.componentInstance.width.set(400);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const handle = part(root, 'handle');
    expect(handle.getAttribute('role')).toBe('separator');
    expect(handle.getAttribute('aria-valuenow')).toBe('400');
    key(handle, 'ArrowLeft');
    expect(fixture.componentInstance.width()).toBe(416);
    key(handle, 'ArrowRight');
    key(handle, 'ArrowRight');
    expect(fixture.componentInstance.width()).toBe(384);
    key(handle, 'End');
    expect(fixture.componentInstance.width()).toBe(720);
    key(handle, 'Home');
    expect(fixture.componentInstance.width()).toBe(280);
    key(handle, 'a');
    expect(fixture.componentInstance.width()).toBe(280);
    await fixture.whenStable();
    expect(root.querySelector('glz-panel-layout')?.getAttribute('style')).toContain(
      '--glz-panel-width: 280px',
    );
  });

  it('resizes as the person drags its edge', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    fixture.componentInstance.width.set(400);
    await fixture.whenStable();
    const handle = part(fixture.nativeElement as HTMLElement, 'handle');
    Object.assign(handle, { setPointerCapture: () => undefined });
    const pointer = (type: string, clientX: number, button = 0): void => {
      const event = new MouseEvent(type, { clientX, button, bubbles: true, cancelable: true });
      Object.assign(event, { pointerId: 1 });
      handle.dispatchEvent(event);
    };
    pointer('pointerdown', 500, 2);
    pointer('pointermove', 450);
    expect(fixture.componentInstance.width()).toBe(400);
    pointer('pointerdown', 500);
    await fixture.whenStable();
    expect(handle.classList).toContain('dragging');
    pointer('pointermove', 450);
    expect(fixture.componentInstance.width()).toBe(450);
    pointer('pointerup', 450);
    pointer('pointermove', 300);
    expect(fixture.componentInstance.width()).toBe(450);
    // Dragged toward the end past half the minimum, it collapses and keeps its width.
    pointer('pointerdown', 500);
    pointer('pointermove', 900);
    pointer('pointerup', 900);
    expect(fixture.componentInstance.open()).toBe(false);
    expect(fixture.componentInstance.width()).toBe(450);
    await fixture.whenStable();
    expect(handle.getAttribute('aria-valuenow')).toBe('0');
    // A click on the collapsed edge brings it back.
    pointer('pointerdown', 900);
    pointer('pointerup', 901);
    expect(fixture.componentInstance.open()).toBe(true);
  });

  it('collapses and restores with Enter, and opens from the collapsed edge by its arrow', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    fixture.componentInstance.width.set(400);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const handle = part(root, 'handle');
    expect(handle.getAttribute('aria-controls')).toBe(part(root, 'panel').id);
    expect(handle.getAttribute('aria-valuemin')).toBe('0');
    expect(handle.getAttribute('aria-valuemax')).toBe('720');
    key(handle, 'Enter');
    expect(fixture.componentInstance.open()).toBe(false);
    await fixture.whenStable();
    // Collapsed, the edge stays, focusable, to bring the panel back.
    expect(part(root, 'handle')).toBe(handle);
    expect(part(root, 'panel').inert).toBe(true);
    key(handle, 'ArrowRight');
    expect(fixture.componentInstance.open()).toBe(false);
    key(handle, 'ArrowLeft');
    expect(fixture.componentInstance.open()).toBe(true);
    expect(fixture.componentInstance.width()).toBe(400);
    key(handle, 'Enter');
    key(handle, 'Enter');
    expect(fixture.componentInstance.open()).toBe(true);
    await fixture.whenStable();
    await expectAccessible(root);
  });

  it('becomes a modal sheet on narrow screens, closed by Escape or the scrim', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    breakpoints.matches.next({ matches: true, breakpoints: {} });
    await fixture.whenStable();
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(part(root, 'panel').getAttribute('role')).toBe('dialog');
    expect(part(root, 'panel').getAttribute('aria-modal')).toBe('true');
    expect(root.querySelector('[data-testid="tools-handle"]')).toBeNull();
    expect(root.querySelector<HTMLElement>('.content')?.inert).toBe(true);
    expect(document.activeElement).toBe(part(root, 'panel'));
    key(part(root, 'panel'), 'Escape');
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(false);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    part(root, 'scrim').click();
    await fixture.whenStable();
    expect(fixture.componentInstance.open()).toBe(false);
  });

  it('with nothing in its panel, is its content alone: no panel, edge or sheet', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    fixture.componentInstance.empty.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(part(root, 'panel').inert).toBe(true);
    expect(getComputedStyle(part(root, 'panel')).display).toBe('none');
    expect(root.querySelector('[data-testid="tools-handle"]')).toBeNull();
    breakpoints.matches.next({ matches: true, breakpoints: {} });
    await fixture.whenStable();
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    expect(part(root, 'panel').getAttribute('aria-modal')).toBeNull();
    expect(root.querySelector('[data-testid="tools-scrim"]')).toBeNull();
    expect(root.querySelector<HTMLElement>('.content')?.inert).toBe(false);
    await expectAccessible(root);
    // Something to show, and it is there as the person left it.
    fixture.componentInstance.empty.set(false);
    await fixture.whenStable();
    expect(part(root, 'panel').getAttribute('aria-modal')).toBe('true');
  });

  it('leaves Escape alone beside the content', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.open.set(true);
    await fixture.whenStable();
    key(part(fixture.nativeElement as HTMLElement, 'panel'), 'Escape');
    expect(fixture.componentInstance.open()).toBe(true);
    fixture.componentInstance.layout().close();
    expect(fixture.componentInstance.open()).toBe(false);
  });
});
