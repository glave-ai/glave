import { DOCUMENT } from '@angular/common';
import { DestroyRef, Service, computed, inject, signal } from '@angular/core';

import {
  type InstallOffer,
  type InstallPrompt,
  installOffer,
  isInstallPrompt,
} from '../lib/install';

/** The display mode of an installed web app, which a browser tab never has. */
const STANDALONE = '(display-mode: standalone)';

/**
 * Whether and how this browser can install glave as an app. Chromium offers its own prompt
 * (`beforeinstallprompt`), kept until the person asks for it; Safari installs by hand (Add to
 * Dock, Add to Home Screen); an installed app (`display-mode: standalone`) offers nothing.
 */
@Service()
export class InstallService {
  private readonly window = inject(DOCUMENT).defaultView;
  private readonly prompt = signal<InstallPrompt | null>(null);
  private readonly installed = signal(this.standalone());

  /** How to offer the install, if at all. */
  readonly offer = computed<InstallOffer>(() =>
    installOffer(this.installed(), this.prompt() !== null, this.window?.navigator.userAgent ?? ''),
  );

  constructor() {
    const window = this.window;
    if (window === null) {
      return;
    }
    const offered = (event: Event): void => {
      if (isInstallPrompt(event)) {
        // Keep the browser's prompt for the person's own click.
        event.preventDefault();
        this.prompt.set(event);
      }
    };
    const done = (): void => {
      this.prompt.set(null);
      this.installed.set(true);
    };
    window.addEventListener('beforeinstallprompt', offered);
    window.addEventListener('appinstalled', done);
    inject(DestroyRef).onDestroy(() => {
      window.removeEventListener('beforeinstallprompt', offered);
      window.removeEventListener('appinstalled', done);
    });
  }

  /** Shows the browser's own install prompt, once: the browser hands out a new one if needed. */
  async install(): Promise<void> {
    const prompt = this.prompt();
    if (prompt === null) {
      return;
    }
    this.prompt.set(null);
    await prompt.prompt();
    const { outcome } = await prompt.userChoice;
    if (outcome === 'accepted') {
      this.installed.set(true);
    }
  }

  private standalone(): boolean {
    try {
      return this.window?.matchMedia(STANDALONE).matches ?? false;
    } catch {
      return false;
    }
  }
}
