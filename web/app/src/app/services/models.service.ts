import { Service, computed, inject, signal } from '@angular/core';
import type { ModelInfo } from '@glave/client';

import { type ModelLabel, defaultModel, modelLabel, modelOptions } from '../lib/models';
import { SessionService } from './session.service';

/**
 * The models this server runs (`model/list`), the default first: what the composer's picker
 * offers, and how each reply names the model that wrote it. Read once, when first asked.
 */
@Service()
export class ModelsService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly ModelInfo[]>([]);
  private pending: Promise<void> | null = null;

  readonly models = this.list.asReadonly();
  /** The model a thread that chose none runs on, or `null` until the list is read. */
  readonly default = computed(() => defaultModel(this.list()));
  readonly options = computed(() => modelOptions(this.list()));
  /** Whether there is a choice to make: more than one model. */
  readonly choosable = computed(() => this.list().length > 1);

  /** Reads the list, once; a failed read is tried again next time. */
  load(): Promise<void> {
    this.pending ??= this.session.call('model/list', {}).then(
      ({ models }) => {
        this.list.set(models);
      },
      () => {
        this.pending = null;
      },
    );
    return this.pending;
  }

  /** How a reply names the model `id`; `fallback` when it is not known. */
  label(id: string | null, fallback: string): ModelLabel {
    return modelLabel(this.list(), id, fallback);
  }
}
