import type { Page } from '@playwright/test';

/** A page with gates in it (`gates`), as a fake's page script sees `window`. */
export interface Gated {
  readonly glaveGates: {
    /** Settles once the test lets `gate` go: at once, when it already has. */
    readonly passed: (gate: string) => Promise<void>;
    /** Lets `gate` go. */
    readonly letGo: (gate: string) => void;
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
    interface Gate {
      readonly passed: Promise<void>;
      readonly letGo: () => void;
    }
    const known = new Map<string, Gate>();
    const gate = (name: string): Gate => {
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
export async function letGo(page: Page, gate: string): Promise<void> {
  await page.evaluate((gate) => {
    (window as unknown as Gated).glaveGates.letGo(gate);
  }, gate);
}
