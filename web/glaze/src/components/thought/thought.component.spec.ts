import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GLZ_THOUGHT_EVERY_MS, GlzThoughtComponent } from './thought.component';

@Component({
  imports: [GlzThoughtComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-thought
    testId="thought"
    [label]="label()"
    [headline]="headline()"
    [live]="live()"
    [selected]="selected()"
    (choose)="chosen.set(chosen() + 1)"
  />`,
})
class HostComponent {
  readonly label = signal('Thinking');
  readonly headline = signal<string | null>(null);
  readonly live = signal(true);
  readonly selected = signal(false);
  readonly chosen = signal(0);
}

function shown(root: HTMLElement): string | null {
  return root.querySelector('[data-testid="thought-headline"]')?.textContent ?? null;
}

describe('GlzThoughtComponent', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('is one line, a button, that says what the model does and its headline', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.headline.set('Checking what runs on port 5173.');
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const line = root.querySelector<HTMLButtonElement>('[data-testid="thought-line"]');
    expect(line?.tagName).toBe('BUTTON');
    expect(line?.type).toBe('button');
    expect(root.querySelector('[data-testid="thought-label"]')?.textContent).toBe('Thinking');
    expect(shown(root)).toBe('Checking what runs on port 5173.');
    // The word shimmers while the model thinks; no arrow, nothing folds.
    expect(line?.classList).toContain('live');
    expect(root.querySelector('details, glz-icon')).toBeNull();
    expect(line?.getAttribute('aria-pressed')).toBe('false');
    await expectAccessible(root);

    line?.click();
    expect(fixture.componentInstance.chosen()).toBe(1);
    fixture.componentInstance.selected.set(true);
    fixture.componentInstance.live.set(false);
    fixture.componentInstance.label.set('Thought for 4s');
    await fixture.whenStable();
    expect(line?.getAttribute('aria-pressed')).toBe('true');
    expect(line?.classList).not.toContain('live');
    expect(root.querySelector('[data-testid="thought-label"]')?.textContent).toBe('Thought for 4s');
    await expectAccessible(root);
  });

  it('shows no headline until it has one', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    expect(shown(fixture.nativeElement as HTMLElement)).toBeNull();
  });

  it('changes its headline at most every 1.5 s while the model thinks, showing the latest', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const fixture = TestBed.createComponent(HostComponent);
    const host = fixture.componentInstance;
    const root: HTMLElement = fixture.nativeElement;
    host.headline.set('One.');
    await fixture.whenStable();
    expect(shown(root)).toBe('One.');
    host.headline.set('Two.');
    await fixture.whenStable();
    host.headline.set('Three.');
    await fixture.whenStable();
    vi.advanceTimersByTime(GLZ_THOUGHT_EVERY_MS - 1);
    await fixture.whenStable();
    expect(shown(root)).toBe('One.');
    // In its turn it fades, then the latest shows: the one between never does.
    vi.advanceTimersByTime(1);
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="thought-headline"]')?.classList).toContain('fading');
    vi.runOnlyPendingTimers();
    await fixture.whenStable();
    expect(shown(root)).toBe('Three.');
    expect(root.querySelector('[data-testid="thought-headline"]')?.classList).not.toContain(
      'fading',
    );
    // Long after, the next shows at its turn, which has come.
    vi.advanceTimersByTime(GLZ_THOUGHT_EVERY_MS * 2);
    host.headline.set('Four.');
    await fixture.whenStable();
    vi.runAllTimers();
    await fixture.whenStable();
    expect(shown(root)).toBe('Four.');
  });

  it('shows the last headline at once when the model is done', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const fixture = TestBed.createComponent(HostComponent);
    const host = fixture.componentInstance;
    host.headline.set('One.');
    await fixture.whenStable();
    host.headline.set('Done.');
    host.live.set(false);
    await fixture.whenStable();
    expect(shown(fixture.nativeElement as HTMLElement)).toBe('Done.');
  });
});
