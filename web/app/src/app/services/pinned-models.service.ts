import { DOCUMENT } from '@angular/common';
import { Service, effect, inject, signal } from '@angular/core';

import { toPinned } from '../lib/models';
import { readStored, storageOf, writeStored } from '../lib/storage';

const KEY = 'glave-pinned-models';

/**
 * The models this person pinned to the top of the model picker, in the order they pinned
 * them. Kept in this browser's storage when it allows; without it they last for the page.
 */
@Service()
export class PinnedModelsService {
  private readonly storage = storageOf(inject(DOCUMENT), 'localStorage');

  /** The pinned models' ids. */
  readonly pinned = signal<readonly string[]>(toPinned(readStored(this.storage, KEY)));

  constructor() {
    effect(() => {
      writeStored(this.storage, KEY, this.pinned());
    });
  }
}
