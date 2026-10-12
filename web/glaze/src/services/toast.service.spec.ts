import { LiveAnnouncer } from '@angular/cdk/a11y';
import { TestBed } from '@angular/core/testing';

import { GlzToastService, MAX_TOASTS } from './toast.service';

describe('GlzToastService', () => {
  let announce: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers();
    announce = vi.fn(() => Promise.resolve());
    TestBed.configureTestingModule({
      providers: [{ provide: LiveAnnouncer, useValue: { announce } }],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('queues a toast and announces it', () => {
    const toasts = TestBed.inject(GlzToastService);
    toasts.show('Saved');
    expect(toasts.toasts().map((toast) => toast.message)).toEqual(['Saved']);
    expect(announce).toHaveBeenCalledWith('Saved', 'polite');
  });

  it('announces danger at once', () => {
    TestBed.inject(GlzToastService).show('Offline', { tone: 'danger' });
    expect(announce).toHaveBeenCalledWith('Offline', 'assertive');
  });

  it('dismisses a plain toast after its time, but keeps one with an action', () => {
    const toasts = TestBed.inject(GlzToastService);
    toasts.show('Copied', { durationMs: 1000 });
    toasts.show('Archived', { action: { label: 'Undo', run: () => undefined } });
    vi.advanceTimersByTime(60_000);
    expect(toasts.toasts().map((toast) => toast.message)).toEqual(['Archived']);
  });

  it('keeps only the latest few', () => {
    const toasts = TestBed.inject(GlzToastService);
    for (let index = 0; index < MAX_TOASTS + 2; index += 1) {
      toasts.show(`Toast ${String(index)}`);
    }
    expect(toasts.toasts()).toHaveLength(MAX_TOASTS);
    expect(toasts.toasts()[0]?.message).toBe('Toast 2');
  });

  it('runs an action once and removes its toast', () => {
    const toasts = TestBed.inject(GlzToastService);
    const run = vi.fn();
    const id = toasts.show('Archived', { action: { label: 'Undo', run } });
    toasts.act(id);
    toasts.act(id);
    expect(run).toHaveBeenCalledOnce();
    expect(toasts.toasts()).toEqual([]);
  });
});
