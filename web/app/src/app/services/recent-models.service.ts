import { DOCUMENT } from '@angular/common';
import { Service, effect, inject, signal } from '@angular/core';

import { toRecent, withRecent } from '../lib/models';
import { readStored, storageOf, writeStored } from '../lib/storage';

const KEY = 'glave-recent-models';

/**
 * The models this person picked lately, the latest first, which the model picker shows after
 * the default. Kept in this browser's storage when it allows; without it they last for the
 * page.
 */
@Service()
export class RecentModelsService {
  private readonly storage = storageOf(inject(DOCUMENT), 'localStorage');
  private readonly ids = signal<readonly string[]>(toRecent(readStored(this.storage, KEY)));

  /** The recent models' ids, at most `RECENT_MODELS`. */
  readonly recent = this.ids.asReadonly();

  constructor() {
    effect(() => {
      writeStored(this.storage, KEY, this.ids());
    });
  }

  /** Notes that the person picked `model`. */
  use(model: string): void {
    this.ids.update((ids) => withRecent(ids, model));
  }
}
