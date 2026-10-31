import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  model,
  signal,
} from '@angular/core';
import { GlzButtonComponent, GlzIconComponent, GlzPickerComponent } from '@glave/glaze';
import { ChevronDown } from 'lucide';

import { modelOptions, orderedOptions } from '../../lib/models';
import { routed } from '../../lib/serving';
import { ModelsService } from '../../services/models.service';
import { RecentModelsService } from '../../services/recent-models.service';
import { ModelDialogComponent } from '../model-dialog/model-dialog.component';

/**
 * The composer's model picker, quiet in its status line: the models this server runs, named
 * `Company · Model` in its list, its button showing the one the next turn goes to by its own
 * name (the person's pick, else the thread's choice, else the default). Searched by name as the person types, so it holds a catalogue of hundreds: the
 * default first, then the models they picked lately, then the rest. `value` is the person's
 * pick, `null` until they make one; the thread sends it with the next turn. Where a router
 * serves any of the models, the button opens the model dialog instead, with the person's
 * filters among the router's providers and who serves each model (`app-model-dialog`).
 */
@Component({
  selector: 'app-model-picker',
  imports: [GlzButtonComponent, GlzIconComponent, GlzPickerComponent, ModelDialogComponent],
  templateUrl: './model-picker.component.html',
  styleUrl: './model-picker.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ModelPickerComponent {
  /** The thread's own choice (its latest `model.chosen`), or `null` for the default. */
  readonly current = input<string | null>(null);
  /** The person's pick, or `null` for none. */
  readonly value = model<string | null>(null);

  protected readonly copy = {
    model: $localize`:@@modelPicker.model:Model`,
    prefix: $localize`:@@modelPicker.prefix:Model:`,
    search: $localize`:@@modelPicker.search:Search models`,
    empty: $localize`:@@modelPicker.empty:No models match`,
  } as const;
  private readonly models = inject(ModelsService);
  private readonly recent = inject(RecentModelsService);
  private readonly locale = inject(LOCALE_ID);
  /** Made here, with the picker, so the first page loads none of the models' names and lines. */
  protected readonly options = computed(() =>
    orderedOptions(
      modelOptions(this.models.models(), this.locale),
      this.models.default(),
      this.recent.recent(),
    ),
  );
  protected readonly hasModels = computed(() => this.options().length > 0);
  /** Whether a router serves any model, so the dialog with its filters picks one. */
  protected readonly filtered = computed(() => this.models.models().some(routed));
  /** The name the button shows: the model's own. */
  protected readonly shownName = computed(() => {
    const id = this.shown();
    return this.options().find((option) => option.value === id)?.short ?? id ?? '';
  });
  protected readonly choosing = signal(false);
  protected readonly open = ChevronDown;
  protected readonly shown = computed(
    () => this.value() ?? this.current() ?? this.models.default(),
  );

  constructor() {
    void this.models.load();
  }

  protected openDialog(): void {
    this.choosing.set(true);
  }

  protected pick(value: string | null): void {
    this.value.set(value);
    if (value !== null) {
      this.recent.use(value);
    }
  }
}
