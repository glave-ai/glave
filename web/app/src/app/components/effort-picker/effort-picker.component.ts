import { ChangeDetectionStrategy, Component, computed, inject, input, model } from '@angular/core';
import { GlzMenuSelectComponent } from '@glave/glaze';

import { defaultEffort, effortOptions } from '../../lib/effort';
import { ModelsService } from '../../services/models.service';

/**
 * How much the next turn's model reasons, quiet in the composer's status line beside the model:
 * shown only when that model offers a choice, with exactly the levels its provider lists, in
 * their names as people read them (`Extra high`, `Off`). The person's pick, else the thread's
 * own (`current`), else the level the server says it sends unasked, else `Default`: none is
 * sent, and the model reasons as its maker set it. `model` is the model the next turn goes to,
 * `null` for the server's default. `value` is the person's pick, `null` until they make one; it
 * goes with the next `thread/start` or `turn/start` when the model offers it.
 */
@Component({
  selector: 'app-effort-picker',
  imports: [GlzMenuSelectComponent],
  templateUrl: './effort-picker.component.html',
  styleUrl: './effort-picker.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class EffortPickerComponent {
  /** The model the next turn goes to, or `null` for the server's default. */
  readonly model = input<string | null>(null);
  /** The thread's own level, or `null` for none. */
  readonly current = input<string | null>(null);
  /** The person's pick, or `null` for none. */
  readonly value = model<string | null>(null);

  protected readonly copy = {
    label: $localize`:@@effortPicker.label:Reasoning`,
    // The button when no level is sent: the model reasons as its maker set it.
    unasked: $localize`:@@effortPicker.unasked:Default`,
  } as const;
  private readonly models = inject(ModelsService);
  private readonly info = computed(() => {
    const id = this.model() ?? this.models.default();
    return this.models.models().find((each) => each.id === id);
  });
  protected readonly options = computed(() => effortOptions(this.info()));
  protected readonly choosable = computed(() => this.options().length > 0);
  /** The level the next turn reasons at: one the model offers. */
  protected readonly shown = computed(() => {
    const offered = this.options().map((option) => option.value);
    const wanted = [this.value(), this.current()].find(
      (level) => level !== null && offered.includes(level),
    );
    return wanted ?? defaultEffort(this.info());
  });

  constructor() {
    void this.models.load();
  }

  protected pick(value: string | null): void {
    this.value.set(value);
  }
}
