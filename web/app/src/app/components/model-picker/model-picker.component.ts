import { ChangeDetectionStrategy, Component, computed, inject, input, model } from '@angular/core';
import { GlzMenuSelectComponent } from '@glave/glaze';

import { ModelsService } from '../../services/models.service';

const COPY = {
  model: 'Model',
} as const;

/**
 * The composer's model picker: the models this server runs, showing the one the next turn
 * goes to (the person's pick, else the thread's choice, else the default). `value` is the
 * person's pick, `null` until they make one; the thread sends it with the next turn.
 */
@Component({
  selector: 'app-model-picker',
  imports: [GlzMenuSelectComponent],
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
  protected readonly options = this.models.options;
  protected readonly hasModels = computed(() => this.options().length > 0);
  protected readonly shown = computed(
    () => this.value() ?? this.current() ?? this.models.default(),
  );

  constructor() {
    void this.models.load();
  }

  protected pick(value: string | null): void {
    this.value.set(value);
  }
}
