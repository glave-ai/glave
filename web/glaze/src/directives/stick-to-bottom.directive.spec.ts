import { ChangeDetectionStrategy, Component, signal, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzStickToBottomDirective } from './stick-to-bottom.directive';

@Component({
  imports: [GlzStickToBottomDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div
      data-testid="list"
      glzStickToBottom
      [(atBottom)]="atBottom"
      (nearTop)="near.set(near() + 1)"
    >
      @for (line of lines(); track line) {
        <p [attr.data-line]="line">{{ line }}</p>
      }
    </div>
  `,
})
class HostComponent {
  readonly lines = signal(['one']);
  readonly atBottom = signal(true);
  /** How many times the person was near the top. */
  readonly near = signal(0);
  readonly stick = viewChild.required(GlzStickToBottomDirective);
}

/** Gives jsdom, which lays nothing out, a scroll box: 1000 high, 200 shown. */
function scrollBox(element: HTMLElement): void {
  Object.defineProperty(element, 'scrollHeight', { configurable: true, value: 1000 });
  Object.defineProperty(element, 'clientHeight', { configurable: true, value: 200 });
}

const flushMutations = (): Promise<void> => new Promise((resolve) => setTimeout(resolve));

describe('GlzStickToBottomDirective', () => {
  it('follows new content while at the bottom, and not after scrolling up', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const list = (fixture.nativeElement as HTMLElement).querySelector<HTMLElement>(
      '[data-testid="list"]',
    );
    if (list === null) {
      throw new Error('no list');
    }
    scrollBox(list);
    fixture.componentInstance.lines.set(['one', 'two']);
    await fixture.whenStable();
    await flushMutations();
    expect(list.scrollTop).toBe(1000);

    list.scrollTop = 100;
    list.dispatchEvent(new Event('scroll'));
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(false);
    fixture.componentInstance.lines.set(['one', 'two', 'three']);
    await fixture.whenStable();
    await flushMutations();
    expect(list.scrollTop).toBe(100);

    fixture.componentInstance.stick().scrollToBottom();
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(true);
    expect(list.scrollTop).toBe(1000);
  });

  it('keeps what the person reads in place while lines come in above it, and says they are near the top', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.lines.set(['three', 'four']);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    const list = root.querySelector<HTMLElement>('[data-testid="list"]');
    if (list === null) {
      throw new Error('no list');
    }
    scrollBox(list);
    // Laid out as a browser would: each line 100 high, one after another, under the scroll.
    const at = (element: Element, top: number, height: number): void => {
      Object.defineProperty(element, 'getBoundingClientRect', {
        configurable: true,
        value: () => ({ top, height }),
      });
    };
    const layOut = (): void => {
      at(list, 0, 200);
      list.querySelectorAll('p').forEach((line, index) => {
        at(line, index * 100 - list.scrollTop, 100);
      });
    };
    list.scrollTop = 150;
    layOut();
    list.dispatchEvent(new Event('scroll'));
    await fixture.whenStable();
    expect(fixture.componentInstance.atBottom()).toBe(false);
    expect(fixture.componentInstance.near()).toBe(1);

    fixture.componentInstance.lines.set(['one', 'two', 'three', 'four']);
    fixture.detectChanges();
    layOut();
    await flushMutations();
    // Two lines came in above: "four", which the person was reading, stays where it was.
    expect(list.scrollTop).toBe(350);
    expect(list.style.overflowAnchor).toBe('none');
  });
});
