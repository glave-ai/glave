import { DOCUMENT } from '@angular/common';
import { Service, effect, inject, signal, untracked } from '@angular/core';
import { type Accent, ApiError, type OrganizationLook, type Shades } from '@glave/client';

import { readStored, forget, storageOf, writeStored } from '../lib/storage';
import { sentence, unreached } from '../lib/text';
import { onChange } from './changes.service';
import { SessionService } from './session.service';

/** The value that stands for glave's plain look among the accents offered. */
export const PLAIN = 'plain';
/** The value that stands for the organization's accent among those offered. */
export const ORGANIZATION = 'organization';

const KEY = 'glave-accent';
const HEX = /^#[0-9a-f]{6}$/u;

/** Whether `value` is the values the server says an accent is drawn in. */
function isShades(value: unknown): value is Shades {
  const shades = value as Partial<Shades> | null;
  return HEX.test(String(shades?.light)) && HEX.test(String(shades?.dark));
}

/** Whether `organization` has an accent to offer. */
export function offered(organization: OrganizationLook | null): boolean {
  return organization !== null && organization.accent !== PLAIN;
}

/** What `value`, {@link PLAIN} or a `#rrggbb`, asks the server to keep. */
function pick(value: string, adjust: boolean): Accent {
  return value === PLAIN ? PLAIN : { color: { color: value, adjust } };
}

/**
 * The person's accent, kept by the server (`appearance/get`, `appearance/set`). The server says
 * the two values to draw it in, light and dark, which go on the page's root as Glaze's tokens,
 * so no component knows where a color came from. They are remembered in this browser too, so
 * the page is drawn in them at once, not first when the server has answered; made as the app
 * starts for that (`appConfig`). A choice shows at once and is sent after the ones before it, so
 * the server keeps the person's last; the server's answer is what stays.
 */
@Service()
export class AppearanceService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  private readonly storage = storageOf(this.document);
  private readonly chosen = signal<string>(PLAIN);
  private readonly tuned = signal(true);
  private readonly failed = signal<string | null>(null);
  private readonly look = signal<OrganizationLook | null>(null);
  /** What the server last said is chosen; none, the organization's. */
  private kept: Accent | null = null;
  /** How many choices the person made in this tab: only the latest one's answer is shown. */
  private choices = 0;
  /** How many choices are on their way. */
  private sendingNow = 0;
  /** The choices on their way, each sent once the one before it is answered. */
  private sending: Promise<unknown> = Promise.resolve();
  /** Whether this tab has been signed in, so that signing out takes the accent away. */
  private signedIn = false;

  /**
   * The accent chosen: {@link PLAIN}, {@link ORGANIZATION}, or its `#rrggbb` as the person chose
   * it.
   */
  readonly value = this.chosen.asReadonly();
  /** Whether a color is tuned so that text stays easy to read. */
  readonly adjust = this.tuned.asReadonly();
  /** Why the last choice was not kept, until the next. */
  readonly problem = this.failed.asReadonly();
  /** What the person's organization says of how glave looks, where it is shown to them. */
  readonly organization = this.look.asReadonly();

  constructor() {
    const remembered = readStored(this.storage, KEY);
    this.draw(isShades(remembered) ? remembered : null);
    onChange('appearance', () => void this.read());
    // Signed out only when it was signed in: as the app starts it is not yet, until the server
    // has answered, and what this browser remembers stays drawn till then.
    effect(() => {
      const signedIn = this.session.signedIn();
      untracked(() => {
        if (signedIn) {
          void this.read();
        } else if (this.signedIn) {
          this.signOut();
        }
        this.signedIn = signedIn;
      });
    });
  }

  /**
   * Keeps `value`, {@link PLAIN}, {@link ORGANIZATION} or a `#rrggbb`, as the person's accent, tuned
   * as `adjust` says.
   */
  async choose(value: string, adjust: boolean): Promise<void> {
    this.choices += 1;
    this.sendingNow += 1;
    const choice = this.choices;
    this.chosen.set(value);
    this.tuned.set(adjust);
    const answered = this.sending.then(() =>
      this.session.call(
        'appearance/set',
        value === ORGANIZATION ? {} : { accent: pick(value, adjust) },
      ),
    );
    this.sending = answered.catch(() => undefined);
    try {
      const { chosen, shades, organization } = await answered;
      this.kept = chosen ?? null;
      if (choice === this.choices) {
        this.failed.set(null);
        this.keep(chosen ?? null, shades ?? null, organization ?? null);
      }
    } catch (error: unknown) {
      if (choice === this.choices) {
        this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
        this.show(this.kept);
      }
    } finally {
      this.sendingNow -= 1;
    }
  }

  /** Whether a color to come is tuned, while there is none for it to tune. */
  tune(adjust: boolean): void {
    this.tuned.set(adjust);
  }

  /** Reads again, as after the person's organization changed what it says. */
  async read(): Promise<void> {
    const choices = this.choices;
    try {
      const { chosen, shades, organization } = await this.session.call('appearance/get', {});
      if (choices === this.choices && this.sendingNow === 0) {
        this.keep(chosen ?? null, shades ?? null, organization ?? null);
      }
    } catch {
      // The page stays as it was drawn; the next change, or the next load, reads it again.
    }
  }

  private keep(
    chosen: Accent | null,
    shades: Shades | null,
    organization: OrganizationLook | null,
  ): void {
    this.kept = chosen;
    this.look.set(organization);
    this.show(chosen);
    this.draw(shades);
    if (shades === null) {
      forget(this.storage, KEY);
    } else {
      writeStored(this.storage, KEY, shades);
    }
  }

  private show(accent: Accent | null): void {
    if (accent === null) {
      this.chosen.set(offered(this.look()) ? ORGANIZATION : PLAIN);
    } else if (accent === PLAIN) {
      this.chosen.set(PLAIN);
    } else {
      this.chosen.set(accent.color.color);
      this.tuned.set(accent.color.adjust);
    }
  }

  /** Back to glave's plain look, and no answer still on its way is shown or remembered. */
  private signOut(): void {
    this.choices += 1;
    this.kept = null;
    this.look.set(null);
    this.chosen.set(PLAIN);
    this.tuned.set(true);
    this.failed.set(null);
    this.draw(null);
    forget(this.storage, KEY);
  }

  /** Puts `shades` on the page's root, or takes them off for Glaze's plain default. */
  private draw(shades: Shades | null): void {
    const { style } = this.document.documentElement;
    if (shades === null) {
      style.removeProperty('--glz-accent-light');
      style.removeProperty('--glz-accent-dark');
    } else {
      style.setProperty('--glz-accent-light', shades.light);
      style.setProperty('--glz-accent-dark', shades.dark);
    }
  }
}
