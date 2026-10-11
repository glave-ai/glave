import { LiveAnnouncer } from '@angular/cdk/a11y';
import { DestroyRef, Service, inject, signal } from '@angular/core';

import type { GlzTone } from '../components/badge/badge.component';

/** An action a toast offers, such as Undo. */
export interface GlzToastAction {
  readonly label: string;
  readonly run: () => void;
}

/** A toast on screen. */
export interface GlzToast {
  readonly id: number;
  readonly message: string;
  readonly tone: GlzTone;
  readonly action: GlzToastAction | null;
}

/** How to show a toast. */
export interface GlzToastOptions {
  readonly tone?: GlzTone;
  /** How long it stays, in milliseconds. A toast with an action stays until dismissed. */
  readonly durationMs?: number;
  readonly action?: GlzToastAction;
}

/** The most toasts on screen at once; older ones give way. */
export const MAX_TOASTS = 3;

/**
 * The queue of brief messages: "Saved", "Copied", "Couldn't reach the server". Each is
 * announced to screen readers (politely, or at once for `danger`); `<glz-toast-stack>`, placed
 * once in the app, shows them. A toast with an action stays until dismissed, so no one has to
 * beat a timer to use it.
 *
 * ```ts
 * this.toasts.show('Thread archived', { action: { label: 'Undo', run: () => this.unarchive() } });
 * ```
 */
@Service()
export class GlzToastService {
  private readonly announcer = inject(LiveAnnouncer);
  private readonly items = signal<readonly GlzToast[]>([]);
  private readonly timers = new Map<number, ReturnType<typeof setTimeout>>();
  private nextId = 1;

  /** The toasts on screen, oldest first. */
  readonly toasts = this.items.asReadonly();

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      for (const timer of this.timers.values()) {
        clearTimeout(timer);
      }
    });
  }

  /** Shows `message`; returns its id, for `dismiss`. */
  show(message: string, options: GlzToastOptions = {}): number {
    const toast: GlzToast = {
      id: this.nextId,
      message,
      tone: options.tone ?? 'neutral',
      action: options.action ?? null,
    };
    this.nextId += 1;
    this.items.update((toasts) => [...toasts, toast].slice(-MAX_TOASTS));
    void this.announcer.announce(message, toast.tone === 'danger' ? 'assertive' : 'polite');
    if (toast.action === null) {
      const timer = setTimeout(() => {
        this.dismiss(toast.id);
      }, options.durationMs ?? 5000);
      this.timers.set(toast.id, timer);
    }
    return toast.id;
  }

  /** Removes the toast with `id`. */
  dismiss(id: number): void {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.items.update((toasts) => toasts.filter((toast) => toast.id !== id));
  }

  /** Runs the toast's action and removes it. */
  act(id: number): void {
    const toast = this.items().find((item) => item.id === id);
    this.dismiss(id);
    toast?.action?.run();
  }
}
