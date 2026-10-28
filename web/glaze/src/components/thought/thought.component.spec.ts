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
    [text]="text"
    [(open)]="open"
  />`,
})
class HostComponent {
  readonly label = signal('Thinking');
  readonly headline = signal<string | null>(null);
  readonly live = signal(true);
  readonly text = 'The lock is taken twice.\nOnce in parse.';
  readonly open = signal(false);
}

function shown(root: HTMLElement): string | null {
  return root.querySelector('[data-testid="thought-headline"]')?.textContent ?? null;
}

describe('GlzThoughtComponent', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('is one quiet line while the model thinks, its word shimmering, with nothing to open', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.headline.set('Checking what runs on port 5173.');
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const line = root.querySelector('[data-testid="thought-line"]');
    expect(line?.tagName).toBe('P');
    expect(line?.classList).toContain('live');
    expect(root.querySelector('[data-testid="thought-label"]')?.textContent).toBe('Thinking');
    expect(shown(root)).toBe('Checking what runs on port 5173.');
    expect(root.querySelector('button, glz-icon, [data-testid="thought-text"]')).toBeNull();
    await expectAccessible(root);
  });

  it('once done, opens the reasoning in full under the line, and shuts it again', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    const host = fixture.componentInstance;
    host.headline.set('Checked it.');
    host.live.set(false);
    host.label.set('Thought for 4s');
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const line = root.querySelector<HTMLButtonElement>('[data-testid="thought-line"]');
    expect(line?.tagName).toBe('BUTTON');
    expect(line?.type).toBe('button');
    expect(line?.classList).not.toContain('live');
    expect(root.querySelector('[data-testid="thought-label"]')?.textContent).toBe('Thought for 4s');
    expect(root.querySelector('[data-testid="thought-chevron"]')).not.toBeNull();
    // It starts shut.
    expect(line?.getAttribute('aria-expanded')).toBe('false');
    expect(root.querySelector('[data-testid="thought-text"]')).toBeNull();
    await expectAccessible(root);

    line?.click();
    await fixture.whenStable();
    expect(host.open()).toBe(true);
    expect(line?.getAttribute('aria-expanded')).toBe('true');
    expect(root.querySelector('[data-testid="thought-text"]')?.textContent).toBe(host.text);
    await expectAccessible(root);

    line?.click();
    await fixture.whenStable();
    expect(host.open()).toBe(false);
    expect(line?.getAttribute('aria-expanded')).toBe('false');
    expect(root.querySelector('[data-testid="thought-text"]')).toBeNull();
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
