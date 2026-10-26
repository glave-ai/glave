import { Service, effect, inject, signal, untracked } from '@angular/core';
import type { Setting } from '@glave/client';

/**
 * What of the person's own the server says changed (`settings/changed`, sent on the lists'
 * watch the side nav keeps): each service that shows one reads it again, so a change made in
 * another tab, or from the terminal, shows here too.
 */
@Service()
export class ChangesService {
  private readonly heard = signal<ReadonlyMap<Setting, number>>(new Map());

  /** Takes the server's word that `setting` changed. */
  tell(setting: Setting): void {
    this.heard.update((heard) => new Map(heard).set(setting, (heard.get(setting) ?? 0) + 1));
  }

  /** How many times `setting` was heard to change. */
  times(setting: Setting): number {
    return this.heard().get(setting) ?? 0;
  }
}

/**
 * Runs `read` each time the server says `setting` changed from now on; called where `inject`
 * works, as a service is made.
 */
export function onChange(setting: Setting, read: () => void): void {
  const changes = inject(ChangesService);
  const from = untracked(() => changes.times(setting));
  effect(() => {
    if (changes.times(setting) > from) {
      untracked(read);
    }
  });
}
