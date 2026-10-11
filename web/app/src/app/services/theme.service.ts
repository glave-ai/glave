import { DOCUMENT } from '@angular/common';
import { Service, effect, inject, signal } from '@angular/core';

/** The person's theme: the system's, or one they picked. */
export type ThemeChoice = 'system' | 'light' | 'dark';

/** The choices in the order the picker moves through them. */
export const THEME_CHOICES: readonly ThemeChoice[] = ['system', 'light', 'dark'];

const KEY = 'glave-theme';

function isChoice(value: unknown): value is ThemeChoice {
  return THEME_CHOICES.some((choice) => choice === value);
}

/** The choice after `choice`: system, light, dark, then system again. */
export function nextTheme(choice: ThemeChoice): ThemeChoice {
  return THEME_CHOICES[(THEME_CHOICES.indexOf(choice) + 1) % THEME_CHOICES.length] ?? 'system';
}

/**
 * The person's light or dark preference. Glaze follows the system with CSS alone; a choice
 * here sets `data-theme` on `<html>`, which Glaze's tokens read. Kept in this browser until
 * the person's settings live on the server.
 */
@Service()
export class ThemeService {
  private readonly document = inject(DOCUMENT);
  private readonly chosen = signal<ThemeChoice>(this.read());

  readonly choice = this.chosen.asReadonly();

  constructor() {
    effect(() => {
      const choice = this.chosen();
      const root = this.document.documentElement;
      if (choice === 'system') {
        root.removeAttribute('data-theme');
      } else {
        root.setAttribute('data-theme', choice);
      }
    });
  }

  set(choice: ThemeChoice): void {
    this.chosen.set(choice);
    try {
      this.document.defaultView?.localStorage.setItem(KEY, choice);
    } catch {
      // Storage may be off, as in some private windows: the choice lasts for this page.
    }
  }

  cycle(): void {
    this.set(nextTheme(this.chosen()));
  }

  private read(): ThemeChoice {
    try {
      const saved = this.document.defaultView?.localStorage.getItem(KEY);
      return isChoice(saved) ? saved : 'system';
    } catch {
      return 'system';
    }
  }
}
