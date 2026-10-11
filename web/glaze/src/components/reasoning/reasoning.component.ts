import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  model,
} from '@angular/core';

/**
 * The model's reasoning, collapsed under one line until the person opens it: a native
 * `<details>`, so it opens with a click, Enter or Space and screen readers say whether it is
 * open. While `streaming`, the line says the model is thinking.
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
})
export class GlzReasoningComponent {
  readonly open = model(false);
  readonly streaming = input(false, { transform: booleanAttribute });
  readonly label = input('Reasoning');
  readonly thinkingLabel = input('Thinking…');

  protected readonly summary = computed(() =>
    this.streaming() ? this.thinkingLabel() : this.label(),
  );

  protected toggled(event: Event): void {
    if (event.target instanceof HTMLDetailsElement) {
      this.open.set(event.target.open);
    }
  }
}
