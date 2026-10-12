import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';
import { injectId } from '@glave/glaze';

import { isLong } from '../../lib/text';

const COPY = {
  more: 'Show more',
  less: 'Show less',
} as const;

/** Past either limit a message folds to its first lines (`--app-folded-lines`, in the styles). */
const FOLD_AFTER_CHARS = 700;
const FOLD_AFTER_LINES = 12;

/**
 * The text of a message, folded when it is long: its first lines, then "Show more", a
 * disclosure button (`aria-expanded`, `aria-controls`) that shows the rest, and "Show less".
 * Short text, or text that is not `foldable`, shows whole, with no button.
 */
@Component({
  selector: 'app-message-text',
  templateUrl: './message-text.component.html',
  styleUrl: './message-text.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MessageTextComponent {
  readonly text = input.required<string>();
  /** Whether long text folds; a reply streaming in, or read in full, does not. */
  readonly foldable = input(true);

  protected readonly textId = injectId('app-message-text');
  protected readonly expanded = signal(false);
  protected readonly long = computed(
    () => this.foldable() && isLong(this.text(), FOLD_AFTER_CHARS, FOLD_AFTER_LINES),
  );
  protected readonly folded = computed(() => this.long() && !this.expanded());
  protected readonly toggleText = computed(() => (this.expanded() ? COPY.less : COPY.more));

  protected toggle(): void {
    this.expanded.update((expanded) => !expanded);
  }
}
