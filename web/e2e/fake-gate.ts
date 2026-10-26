import type { Page } from '@playwright/test';

/**
 * The gates the fakes' streams wait at: `streamReply`'s `reply`, `steerTurn`'s `read`, and
 * `thinkingTurn`'s steps.
 */
export type Gate = 'reply' | 'read' | 'start' | 'tool' | 'think' | 'answer';

/**
 * A page with gates in it (`gates`), as a fake's page script sees `window`. The order the page's
 * init scripts run in is not defined, so a fake looks its gate up once its stream comes to it.
 */
export interface Gated {
  readonly glaveGates: {
    /** Settles once the test lets `gate` go: at once, when it already has. */
    readonly passed: (gate: Gate) => Promise<void>;
    /** Lets `gate` go. */
    readonly letGo: (gate: Gate) => void;
  };
}

/**
 * Puts gates in the page, before any script of its own runs, for a fake's stream to wait at
 * (`glaveGates.passed`) until the test lets it go (`letGo`), as a slow model would: so what
 * the page shows partway through a turn stays for as long as the test looks, with no timer to
 * race, as `hold` does for a call on its way. A gate is known by its name, and opens once; one
 * let go before the stream comes to it is passed straight through.
 */
export async function gates(page: Page): Promise<void> {
  await page.addInitScript(() => {
    /** A gate as the page keeps it: passed once let go. */
    interface Passing {
      readonly passed: Promise<void>;
      readonly letGo: () => void;
    }
    const known = new Map<Gate, Passing>();
    const gate = (name: Gate): Passing => {
      const found = known.get(name);
      if (found !== undefined) {
        return found;
      }
      let letGo = (): void => undefined;
      const passed = new Promise<void>((resolve) => {
        letGo = resolve;
      });
      const made = { passed, letGo };
      known.set(name, made);
      return made;
    };
    const glaveGates: Gated['glaveGates'] = {
      passed: (name) => gate(name).passed,
      letGo: (name) => {
        gate(name).letGo();
      },
    };
    Object.assign(window, { glaveGates });
  });
}

/** Lets the page's `gate` go, and its fake's stream on past it. */
export async function letGo(page: Page, gate: Gate): Promise<void> {
  await page.evaluate((gate) => {
    (window as unknown as Gated).glaveGates.letGo(gate);
  }, gate);
}
