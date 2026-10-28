// Unit tests run in jsdom, which has no Web Animations API and no motion tokens. These give a
// test the slow duration (or none, as when the person asks for less motion) and a clock it
// moves by hand: the eased progress of every animation timed, and the frames that read it.

/** The animation the test moves: its progress, eased, from 0 to 1. */
interface Timed {
  progress: number;
  finished: boolean;
}

/** What a test of something timed by the motion tokens moves. */
export interface FakeMotion {
  /** Moves every animation to `progress`, eased, and runs the frames that wait for it. */
  readonly to: (progress: number) => void;
  /** Ends every animation, and runs the frames that wait for it. */
  readonly end: () => void;
  /** Puts things back as they were. */
  readonly restore: () => void;
}

/**
 * Sets the slow duration to `slow` (`'0ms'` as when the person asks for less motion) and puts
 * a fake Web Animations API on the window, moved by hand.
 */
export function fakeMotion(slow: string): FakeMotion {
  const root = document.documentElement;
  root.style.setProperty('--glz-duration-slow', slow);
  root.style.setProperty('--glz-ease-out', 'cubic-bezier(0.2, 0, 0, 1)');
  const timed: Timed[] = [];
  let frames = new Map<number, FrameRequestCallback>();
  let next = 0;
  class FakeEffect {
    readonly timing: Timed = { progress: 0, finished: false };
    constructor() {
      timed.push(this.timing);
    }
    getComputedTiming(): { progress: number } {
      return { progress: this.timing.progress };
    }
  }
  class FakeAnimation {
    constructor(private readonly effect: FakeEffect) {}
    get playState(): string {
      return this.effect.timing.finished ? 'finished' : 'running';
    }
    play(): void {
      // Started by the test's clock.
    }
    cancel(): void {
      this.effect.timing.finished = true;
    }
  }
  const run = (): void => {
    const waiting = frames;
    frames = new Map();
    for (const frame of waiting.values()) {
      frame(0);
    }
  };
  vi.stubGlobal('KeyframeEffect', FakeEffect);
  vi.stubGlobal('Animation', FakeAnimation);
  // Angular's scheduler asks for frames too, and cancels its own.
  vi.stubGlobal('requestAnimationFrame', (frame: FrameRequestCallback) => {
    next += 1;
    frames.set(next, frame);
    return next;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => {
    frames.delete(id);
  });
  return {
    to: (progress) => {
      for (const each of timed) {
        each.progress = progress;
      }
      run();
    },
    end: () => {
      for (const each of timed) {
        each.finished = true;
      }
      run();
    },
    restore: () => {
      vi.unstubAllGlobals();
      root.style.removeProperty('--glz-duration-slow');
      root.style.removeProperty('--glz-ease-out');
    },
  };
}
