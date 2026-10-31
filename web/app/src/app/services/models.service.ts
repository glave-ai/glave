import { Service, computed, inject, signal } from '@angular/core';
import type { ModelInfo } from '@glave/client';

import { onChange } from './changes.service';
import { SessionService } from './session.service';

/**
 * The models this server runs (`model/list`), the default first: what the composer's picker
 * offers, each with who serves it through a router and which of those the person's filters
 * hide. Read once, when first asked, and again when the model accounts or the filters change,
 * here or, as the server says, anywhere else.
 */
@Service()
export class ModelsService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly ModelInfo[]>([]);
  private pending: Promise<void> | null = null;
  /** How many reads were begun: only the latest one's answer is shown. */
  private reads = 0;

  readonly models = this.list.asReadonly();
  /** The model a thread that chose none runs on, or `null` until the list is read. */
  readonly default = computed(() => {
    // The one the server marks, else its first. Here rather than with the models' names
    // (lib/models.ts), which only the picker needs: the first page loads none of them.
    const models = this.list();
    return (models.find((model) => model.default) ?? models.at(0))?.id ?? null;
  });
  /** Whether there is a choice to make: more than one model. */
  readonly choosable = computed(() => this.list().length > 1);

  constructor() {
    onChange('providers', () => void this.reload());
    // Which of a model's providers the person's filters hide is the server's word.
    onChange('filters', () => void this.reload());
  }

  /** Reads the list, once; a failed read is tried again next time. */
  load(): Promise<void> {
    if (this.pending === null) {
      // Reads may overlap, as the filters change twice: the latest one's answer stands.
      this.reads += 1;
    }
    const read = this.reads;
    this.pending ??= this.session.call('model/list', {}).then(
      ({ models }) => {
        if (read === this.reads) {
          this.list.set(models);
        }
      },
      () => {
        this.pending = null;
      },
    );
    return this.pending;
  }

  /** Reads the list again, as after a model account was connected or removed. */
  reload(): Promise<void> {
    this.pending = null;
    return this.load();
  }

  /** Whether the server runs no model yet, once the list is read; `false` when it cannot be. */
  async none(): Promise<boolean> {
    await this.load();
    return this.pending !== null && this.list().length === 0;
  }
}
