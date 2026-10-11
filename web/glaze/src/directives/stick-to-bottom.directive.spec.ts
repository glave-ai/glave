import { ChangeDetectionStrategy, Component, signal, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzStickToBottomDirective } from './stick-to-bottom.directive';

@Component({
  imports: [GlzStickToBottomDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div data-testid="list" glzStickToBottom [(atBottom)]="atBottom">
      @for (line of lines(); track $index) {
        <p>{{ line }}</p>
      }
    </div>
  `,
})
class HostComponent {
  readonly lines = signal(['one']);
  readonly atBottom = signal(true);
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
});
