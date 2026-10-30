import { Service, inject, signal } from '@angular/core';
import { ApiError, type Filters } from '@glave/client';

import { sentence, unreached } from '../lib/text';
import { onChange } from './changes.service';
import { ModelsService } from './models.service';
import { SessionService } from './session.service';

/**
 * The person's filters among the providers a router (OpenRouter) passes their calls on to,
 * kept by the server (`filters/get`, `filters/set`) and kept to by every call of their work,
 * and what "Europe" means among them. Read when first asked, and again when the server says
 * they changed elsewhere. Until a read has answered, `known` is false and nothing may be
 * chosen: a choice sends the whole of the filters, so one made on filters never read would
 * replace what the person kept. A choice shows at once and is sent after the ones before it,
 * so the server keeps the person's last; the server's answer is what stays, and the models are
 * read again with which providers it now hides. A read that answers while a choice is on its
 * way never undoes it. When the server keeps none, what it last kept comes back, with why.
 */
@Service()
export class FiltersService {
  private readonly session = inject(SessionService);
  private readonly models = inject(ModelsService);
  private readonly chosen = signal<Filters>({});
  private readonly places = signal<readonly string[]>([]);
  private readonly read = signal(false);
  private readonly failed = signal<string | null>(null);
  /** What the server last said it keeps. */
  private kept: Filters = {};
  /** How many choices the person made in this tab: only the latest one's answer is shown. */
  private choices = 0;
  /** How many choices are on their way. */
  private sendingNow = 0;
  /** The choices on their way, each sent once the one before it is answered. */
  private sending: Promise<unknown> = Promise.resolve();
  private pending: Promise<void> | null = null;

  /** The person's filters. */
  readonly filters = this.chosen.asReadonly();
  /** The countries "Europe" means among them, as the server says. */
  readonly europe = this.places.asReadonly();
  /** Whether the server has said what they are, so they may be chosen. */
  readonly known = this.read.asReadonly();
  /** Why they could not be read, or the last choice was not kept, until the next. */
  readonly problem = this.failed.asReadonly();

  constructor() {
    onChange('filters', () => {
      this.pending = null;
      void this.load();
    });
  }

  /** Reads the person's filters, once; one that cannot be read says so, and is tried again. */
  load(): Promise<void> {
    const choices = this.choices;
    this.pending ??= this.session.call('filters/get', {}).then(
      ({ filters, europe }) => {
        this.places.set(europe);
        this.kept = filters;
        if (choices === this.choices && this.sendingNow === 0) {
          this.chosen.set(filters);
          this.read.set(true);
          this.failed.set(null);
        }
      },
      (error: unknown) => {
        this.pending = null;
        if (!this.read()) {
          this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
        }
      },
    );
    return this.pending;
  }

  /**
   * Keeps `filters` as the person's, showing them at once, and reads the models again: true
   * once the server keeps them.
   */
  async choose(filters: Filters): Promise<boolean> {
    if (!this.read()) {
      return false;
    }
    this.choices += 1;
    this.sendingNow += 1;
    const choice = this.choices;
    this.chosen.set(filters);
    const answered = this.sending.then(() => this.session.call('filters/set', { filters }));
    this.sending = answered.catch(() => undefined);
    try {
      const { filters: kept, europe } = await answered;
      this.kept = kept;
      this.places.set(europe);
      if (choice === this.choices) {
        this.chosen.set(kept);
        this.failed.set(null);
        await this.models.reload();
      }
      return true;
    } catch (error: unknown) {
      if (choice === this.choices) {
        this.chosen.set(this.kept);
        this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
      }
      return false;
    } finally {
      this.sendingNow -= 1;
    }
  }
}
