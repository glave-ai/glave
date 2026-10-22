import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  model,
} from '@angular/core';

import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { summary: 'summary', text: 'text' } as const;

/**
 * The model's reasoning, collapsed under one line until the person opens it: a native
 * `<details>`, so it opens with a click, Enter or Space and screen readers say whether it is
 * open. While `streaming`, the line says the model is thinking. Its text is drawn as written,
 * line breaks kept, so leave out the blank lines at its ends, which would show as space.
 *
 * ```html
 * <glz-reasoning [streaming]="thinking()">{{ reasoning() }}</glz-reasoning>
 * ```
 */
@Component({
  selector: 'glz-reasoning',
  templateUrl: './reasoning.component.html',
  styleUrl: './reasoning.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzReasoningComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('reasoning', {
    label: 'Reasoning',
    thinkingLabel: 'Thinking…',
  });

  readonly open = model(false);
  readonly streaming = input(false, { transform: booleanAttribute });
  readonly label = input(this.words.label);
  readonly thinkingLabel = input(this.words.thinkingLabel);

  protected readonly summary = computed(() =>
    this.streaming() ? this.thinkingLabel() : this.label(),
  );

  protected toggled(event: Event): void {
    if (event.target instanceof HTMLDetailsElement) {
      this.open.set(event.target.open);
    }
  }
}
