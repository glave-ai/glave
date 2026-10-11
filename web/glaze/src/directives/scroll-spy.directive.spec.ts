import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzScrollSpyDirective } from './scroll-spy.directive';

@Component({
  imports: [GlzScrollSpyDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <main glzScrollSpy [(active)]="active">
      <section id="forms">Forms</section>
      <section id="overlays">Overlays</section>
    </main>
  `,
})
class HostComponent {
  readonly active = signal<string | null>(null);
}

/** An IntersectionObserver the test drives: `report` says which elements are in view. */
class FakeObserver {
  static last: FakeObserver | undefined;
  readonly watched: Element[] = [];
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeObserver.last = this;
  }
  observe(target: Element): void {
    this.watched.push(target);
  }
  disconnect(): void {
    this.watched.length = 0;
  }
  report(inView: readonly string[]): void {
    const entries = this.watched.map(
      (target) =>
        ({ target, isIntersecting: inView.includes(target.id) }) as IntersectionObserverEntry,
    );
    this.callback(entries, this as unknown as IntersectionObserver);
  }
}

describe('GlzScrollSpyDirective', () => {
  beforeEach(() => {
    vi.stubGlobal('IntersectionObserver', FakeObserver);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('names the first section in view', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const observer = FakeObserver.last;
    expect(observer?.watched.map((section) => section.id)).toEqual(['forms', 'overlays']);
    observer?.report(['overlays']);
    expect(fixture.componentInstance.active()).toBe('overlays');
    observer?.report(['forms', 'overlays']);
    expect(fixture.componentInstance.active()).toBe('forms');
  });
});
