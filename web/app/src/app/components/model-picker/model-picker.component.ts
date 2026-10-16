import { ChangeDetectionStrategy, Component, computed, inject, input, model } from '@angular/core';
import { GlzPickerComponent } from '@glave/glaze';

import { orderedOptions } from '../../lib/models';
import { ModelsService } from '../../services/models.service';
import { RecentModelsService } from '../../services/recent-models.service';

const COPY = {
  model: 'Model',
  search: 'Search models',
  empty: 'No models match',
} as const;

/**
 * The composer's model picker, quiet in its status line: the models this server runs, named
 * `Company · Model` in its list, its button showing the one the next turn goes to by its own
 * name (the person's pick, else the thread's choice, else the default). Searched by name as the person types, so it holds a catalogue of hundreds: the
 * default first, then the models they picked lately, then the rest. `value` is the person's
 * pick, `null` until they make one; the thread sends it with the next turn.
 */
@Component({
  selector: 'app-model-picker',
  imports: [GlzPickerComponent],
  templateUrl: './model-picker.component.html',
  styleUrl: './model-picker.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ModelPickerComponent {
  /** The thread's own choice (its latest `model.chosen`), or `null` for the default. */
  readonly current = input<string | null>(null);
  /** The person's pick, or `null` for none. */
  readonly value = model<string | null>(null);

  protected readonly copy = COPY;
  private readonly models = inject(ModelsService);
  private readonly recent = inject(RecentModelsService);
  protected readonly options = computed(() =>
    orderedOptions(this.models.options(), this.models.default(), this.recent.recent()),
  );
  protected readonly hasModels = computed(() => this.options().length > 0);
  protected readonly shown = computed(
    () => this.value() ?? this.current() ?? this.models.default(),
  );

  constructor() {
    void this.models.load();
  }

  protected pick(value: string | null): void {
    this.value.set(value);
    if (value !== null) {
      this.recent.use(value);
    }
  }
}
