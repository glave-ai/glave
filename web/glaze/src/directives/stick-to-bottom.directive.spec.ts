import { ChangeDetectionStrategy, Component, signal, viewChild } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';

import { GlzStickToBottomDirective } from './stick-to-bottom.directive';

@Component({
  imports: [GlzStickToBottomDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div data-testid="list" glzStickToBottom [(atBottom)]="atBottom">
      @for (line of lines(); track $index) {
        <p>{{ line }}</p>
      }
      <details>
        <summary>Steps</summary>
        write src/lock.rs
      </details>
    </div>
  `,
})
class HostComponent {
  readonly lines = signal(['one']);
  readonly atBottom = signal(true);
  readonly stick = viewChild.required(GlzStickToBottomDirective);
}

/**
 * jsdom has no ResizeObserver, and lays nothing out: this one keeps what it watches, and says
 * sizes changed when a test does (`resized`).
 */
class FakeResizeObserver {
  static readonly made: FakeResizeObserver[] = [];
  readonly watched = new Set<Element>();

  constructor(readonly changed: () => void) {
    FakeResizeObserver.made.push(this);
  }

  observe(target: Element): void {
    this.watched.add(target);
  }

  unobserve(target: Element): void {
    this.watched.delete(target);
  }

  disconnect(): void {
    this.watched.clear();
  }
}

/** Says, as a browser does once it has laid the page out again, that sizes changed. */
function resized(): void {
  for (const observer of FakeResizeObserver.made) {
    if (observer.watched.size > 0) {
      observer.changed();
    }
  }
}

/** Gives jsdom a scroll box `height` high, 200 of it shown. */
function scrollBox(element: HTMLElement, height: number): void {
  Object.defineProperty(element, 'scrollHeight', { configurable: true, value: height });
  Object.defineProperty(element, 'clientHeight', { configurable: true, value: 200 });
}

/** Scrolls to `top`, as the person (or anything else) does. */
function scrollTo(element: HTMLElement, top: number): void {
  element.scrollTop = top;
  element.dispatchEvent(new Event('scroll'));
}

const flushMutations = (): Promise<void> => new Promise((resolve) => setTimeout(resolve));

async function open(): Promise<{ fixture: ComponentFixture<HostComponent>; list: HTMLElement }> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  await flushMutations();
  const list = (fixture.nativeElement as HTMLElement).querySelector<HTMLElement>(
    '[data-testid="list"]',
  );
  if (list === null) {
    throw new Error('no list');
  }
  return { fixture, list };
}

describe('GlzStickToBottomDirective', () => {
  beforeEach(() => {
    FakeResizeObserver.made.length = 0;
    vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('follows new content while at the bottom, and not after scrolling up', async () => {
    const { fixture, list } = await open();
    scrollBox(list, 1000);
    fixture.componentInstance.lines.set(['one', 'two']);
    await fixture.whenStable();
    await flushMutations();
    expect(list.scrollTop).toBe(1000);

    scrollTo(list, 100);
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(false);
    fixture.componentInstance.lines.set(['one', 'two', 'three']);
    await fixture.whenStable();
    await flushMutations();
    scrollBox(list, 1200);
    resized();
    expect(list.scrollTop).toBe(100);

    fixture.componentInstance.stick().scrollToBottom();
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(true);
    expect(list.scrollTop).toBe(1200);
  });

  it('keeps the person at the bottom when something there opens, so a small scroll after finds them still there', async () => {
    const { fixture, list } = await open();
    scrollBox(list, 1000);
    scrollTo(list, 800);
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(true);

    // The steps open: 40 pixels more, and nothing added or removed.
    const details = list.querySelector('details');
    if (details === null) {
      throw new Error('no details');
    }
    details.open = true;
    scrollBox(list, 1040);
    resized();
    expect(list.scrollTop).toBe(1040);

    // Bringing what opened into view scrolls the few pixels left, which the browser holds at
    // the bottom (1040 - 200); "Jump to latest" stays away. Left at 800, the same 8 pixels would
    // have put the person 32 from the bottom, past the threshold.
    scrollTo(list, 840);
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(true);
  });

  it('watches what it holds as it comes and goes, and itself', async () => {
    const { fixture, list } = await open();
    const [sizes] = FakeResizeObserver.made;
    if (sizes === undefined) {
      throw new Error('nothing watches sizes');
    }
    const lines = (): HTMLElement[] => Array.from(list.querySelectorAll('p'));
    expect(sizes.watched.has(list)).toBe(true);
    expect(lines().every((line) => sizes.watched.has(line))).toBe(true);

    fixture.componentInstance.lines.set(['one', 'two']);
    await fixture.whenStable();
    await flushMutations();
    const added = lines().at(-1);
    if (added === undefined) {
      throw new Error('no line added');
    }
    expect(sizes.watched.has(added)).toBe(true);

    fixture.componentInstance.lines.set(['one']);
    await fixture.whenStable();
    await flushMutations();
    expect(sizes.watched.has(added)).toBe(false);

    fixture.destroy();
    expect(sizes.watched.size).toBe(0);
  });
});
