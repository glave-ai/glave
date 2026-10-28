import { DOCUMENT } from '@angular/common';
import { Service, inject, signal } from '@angular/core';

import { readStored, storageOf, writeStored } from '../lib/storage';

/** The person's theme: the system's, or one they picked. */
type ThemeChoice = 'system' | 'light' | 'dark';

/** The choices, in the order Settings lists them. */
const THEME_CHOICES: readonly ThemeChoice[] = ['system', 'light', 'dark'];

const KEY = 'glave-theme';

/** Whether `value` is one of the theme choices. */
export function isChoice(value: unknown): value is ThemeChoice {
  return THEME_CHOICES.some((choice) => choice === value);
}

/**
 * The person's light or dark preference. Glaze follows the system with CSS alone; a choice
 * here sets `data-theme` on `<html>`, which Glaze's tokens read. Kept in this browser until
 * the person's settings live on the server. Made as the app starts (`appConfig`), so a choice
 * is applied before anything is drawn, not first when Settings opens.
 */
@Service()
export class ThemeService {
  private readonly document = inject(DOCUMENT);
  private readonly storage = storageOf(this.document);
  private readonly chosen = signal<ThemeChoice>(this.read());

  readonly choice = this.chosen.asReadonly();

  constructor() {
    this.apply(this.chosen());
  }

  set(choice: ThemeChoice): void {
    this.chosen.set(choice);
    writeStored(this.storage, KEY, choice);
    this.apply(choice);
  }

  /** Sets `data-theme` on `<html>` at once, so the page is drawn in it. */
  private apply(choice: ThemeChoice): void {
    const root = this.document.documentElement;
    if (choice === 'system') {
      root.removeAttribute('data-theme');
    } else {
      root.setAttribute('data-theme', choice);
    }
  }

  private read(): ThemeChoice {
    const saved = readStored(this.storage, KEY);
    return isChoice(saved) ? saved : 'system';
  }
}
