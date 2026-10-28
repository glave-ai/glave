import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import { GlzCardComponent, formatDuration } from '@glave/glaze';

import { ThoughtService } from '../../services/thought.service';

/**
 * The model's reasoning the person chose to read in full, from its line in the conversation, a
 * card of the thread's panel while one is chosen: how long it took beside its heading, as the
 * server said, then the text as the model wrote it, its lines kept.
 */
@Component({
  selector: 'app-panel-thought',
  imports: [GlzCardComponent],
  templateUrl: './panel-thought.component.html',
  styleUrl: './panel-thought.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PanelThoughtComponent {
  private readonly thoughts = inject(ThoughtService);
  private readonly locale = inject(LOCALE_ID);

  protected readonly copy = { heading: $localize`:@@panelThought.heading:Reasoning` } as const;
  /** The reasoning shown, and how long it took for the reader; `null` while none is chosen. */
  protected readonly shown = computed(() => {
    const shown = this.thoughts.shown();
    const ms = shown?.thought.ms ?? null;
    return shown === null
      ? null
      : { text: shown.text, took: ms === null ? null : formatDuration(ms, this.locale) };
  });
}
