import { LiveAnnouncer } from '@angular/cdk/a11y';
import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzWorkingComponent } from './working.component';

/** How long each word shows. */
const WORD_MS = 2000;
const WORDS = ['Forging…', 'Honing…', 'Tempering…'];

@Component({
  imports: [GlzWorkingComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-working testId="plain" />
    <glz-working testId="marked" label="Thinking…">
      <span glzWorkingMark data-testid="mark">g</span>
    </glz-working>
  `,
})
class MarksComponent {}

@Component({
  imports: [GlzWorkingComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (shown()) {
      <glz-working
        testId="working"
        label="Thinking…"
        [words]="words"
        [step]="step()"
        [elapsed]="elapsed()"
      />
    }
  `,
})
class HostComponent {
  readonly shown = signal(true);
  readonly words = WORDS;
  readonly step = signal<string | null>(null);
  readonly elapsed = signal<string | null>(null);
}

function announcing(): ReturnType<typeof vi.fn> {
  const announce = vi.fn(() => Promise.resolve());
  TestBed.configureTestingModule({
    providers: [{ provide: LiveAnnouncer, useValue: { announce } }],
  });
  return announce;
}

async function render(): Promise<ComponentFixture<HostComponent>> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  return fixture;
}

function line(fixture: ComponentFixture<HostComponent>): string | null | undefined {
  const root: HTMLElement = fixture.nativeElement;
  return root.querySelector('[data-testid="working-label"]')?.textContent;
}

/** Lets `ms` pass, then lets the page catch up. */
async function wait(fixture: ComponentFixture<HostComponent>, ms: number): Promise<void> {
  vi.advanceTimersByTime(ms);
  await fixture.whenStable();
}

describe('GlzWorkingComponent', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('says work is going on, announces it politely, and takes a mark of its own', async () => {
    const announce = announcing();
    const fixture = TestBed.createComponent(MarksComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const plain = root.querySelector('[data-testid="plain"]');
    expect(root.querySelector('[data-testid="plain-label"]')?.textContent).toBe('Working…');
    expect(plain?.querySelector('.dot')).not.toBeNull();
    expect(plain?.querySelector('.mark')?.getAttribute('aria-hidden')).toBe('true');
    const marked = root.querySelector('[data-testid="marked"]');
    expect(marked?.querySelector('[data-testid="mark"]')).not.toBeNull();
    expect(marked?.querySelector('.dot')).toBeNull();
    expect(announce).toHaveBeenCalledWith('Thinking…', 'polite');
    expect(announce).toHaveBeenCalledWith('Working…', 'polite');
    // With one word or none, nothing moves on.
    expect(vi.getTimerCount()).toBe(0);
    await expectAccessible(root);
  });

  it('shows its words in turn, every couple of seconds, while screen readers hear the label once', async () => {
    const announce = announcing();
    const fixture = await render();
    expect(line(fixture)).toBe('Forging…');
    await wait(fixture, WORD_MS - 1);
    expect(line(fixture)).toBe('Forging…');
    await wait(fixture, 1);
    expect(line(fixture)).toBe('Honing…');
    await wait(fixture, WORD_MS);
    expect(line(fixture)).toBe('Tempering…');
    await wait(fixture, WORD_MS);
    expect(line(fixture)).toBe('Forging…');
    expect(announce.mock.calls).toEqual([['Thinking…', 'polite']]);
    await expectAccessible(fixture.nativeElement);
  });

  it('shows the step being taken in the words’ place, announced, and the words again after', async () => {
    const announce = announcing();
    const fixture = await render();
    await wait(fixture, WORD_MS);
    fixture.componentInstance.step.set('bash: cargo test');
    await fixture.whenStable();
    expect(line(fixture)).toBe('bash: cargo test');
    expect(announce).toHaveBeenLastCalledWith('bash: cargo test', 'polite');
    // The words rest while a step shows.
    expect(vi.getTimerCount()).toBe(0);
    await wait(fixture, WORD_MS * 3);
    expect(line(fixture)).toBe('bash: cargo test');
    fixture.componentInstance.step.set('read src/lock.rs');
    await fixture.whenStable();
    expect(line(fixture)).toBe('read src/lock.rs');
    expect(announce).toHaveBeenLastCalledWith('read src/lock.rs', 'polite');
    // Back to thinking: the words go on from where they were, unannounced.
    fixture.componentInstance.step.set(null);
    await fixture.whenStable();
    expect(line(fixture)).toBe('Honing…');
    await wait(fixture, WORD_MS);
    expect(line(fixture)).toBe('Tempering…');
    // A step of many lines, such as a script, is heard by its first.
    fixture.componentInstance.step.set("bash: cat <<'EOF' > notes.md\n# Notes\nEOF");
    await fixture.whenStable();
    expect(announce.mock.calls).toEqual([
      ['Thinking…', 'polite'],
      ['bash: cargo test', 'polite'],
      ['read src/lock.rs', 'polite'],
      ["bash: cat <<'EOF' > notes.md", 'polite'],
    ]);
    await expectAccessible(fixture.nativeElement);
  });

  it('shows how long the work has gone on after the line, once there is a time', async () => {
    announcing();
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    const elapsed = (): Element | null => root.querySelector('[data-testid="working-elapsed"]');
    expect(elapsed()).toBeNull();
    fixture.componentInstance.elapsed.set('12s');
    await fixture.whenStable();
    expect(elapsed()?.textContent).toBe('12s');
    expect(root.querySelector('[data-testid="working-label"]')?.nextElementSibling).toBe(elapsed());
    await expectAccessible(root);
  });

  it('still changes its words when the person asks for less motion, which they move nothing of', async () => {
    announcing();
    vi.stubGlobal('matchMedia', (query: string): Pick<MediaQueryList, 'matches' | 'media'> => ({
      matches: query.includes('prefers-reduced-motion: reduce'),
      media: query,
    }));
    const fixture = await render();
    expect(line(fixture)).toBe('Forging…');
    await wait(fixture, WORD_MS);
    expect(line(fixture)).toBe('Honing…');
  });

  it('leaves no timer behind once it goes', async () => {
    announcing();
    const fixture = await render();
    expect(vi.getTimerCount()).toBe(1);
    fixture.componentInstance.shown.set(false);
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).querySelector('glz-working')).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
    const again = await render();
    expect(vi.getTimerCount()).toBe(1);
    again.destroy();
    expect(vi.getTimerCount()).toBe(0);
  });
});
