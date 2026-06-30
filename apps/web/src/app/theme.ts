import { DOCUMENT } from '@angular/common';
import { inject, Injectable, signal } from '@angular/core';

type Theme = 'light' | 'dark';
const STORAGE_KEY = 'hope-theme';

/**
 * App theme. Light-first by design — Hope greets you in dawn light, never the
 * machine. Dark (warm dusk) is an opt-in we remember; we deliberately do NOT
 * follow the OS preference, so a dark Mac still gets the warm welcome.
 */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  private readonly doc = inject(DOCUMENT);
  readonly dark = signal(this.read() === 'dark');

  constructor() {
    this.apply(this.dark() ? 'dark' : 'light');
  }

  toggle(): void {
    const next: Theme = this.dark() ? 'light' : 'dark';
    this.dark.set(next === 'dark');
    this.apply(next);
    this.doc.defaultView?.localStorage?.setItem(STORAGE_KEY, next);
  }

  private apply(theme: Theme): void {
    this.doc.documentElement.setAttribute('data-theme', theme);
  }

  private read(): Theme {
    return this.doc.defaultView?.localStorage?.getItem(STORAGE_KEY) === 'dark' ? 'dark' : 'light';
  }
}
