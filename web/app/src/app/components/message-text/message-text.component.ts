import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';
import { GlzMarkdownComponent, injectId } from '@glave/glaze';

import { isLong } from '../../lib/text';

/** Past either limit a message folds to its first lines (`--app-folded-lines`, in the styles). */
const FOLD_AFTER_CHARS = 700;
const FOLD_AFTER_LINES = 12;

/**
 * The text of a message: a reply's as markdown, read in full; the person's as they typed it,
 * folded when it is long: its first lines, then "Show more", a disclosure button
 * (`aria-expanded`, `aria-controls`) that shows the rest, and "Show less". Short text shows
 * whole, with no button.
 */
@Component({
  selector: 'app-message-text',
  imports: [GlzMarkdownComponent],
  templateUrl: './message-text.component.html',
  styleUrl: './message-text.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MessageTextComponent {
  readonly text = input.required<string>();
  /** Whether the text is markdown, as a reply's is. */
  readonly markdown = input(false);

  protected readonly textId = injectId('app-message-text');
  protected readonly expanded = signal(false);
  protected readonly long = computed(
    () => !this.markdown() && isLong(this.text(), FOLD_AFTER_CHARS, FOLD_AFTER_LINES),
  );
  protected readonly folded = computed(() => this.long() && !this.expanded());
  private readonly copy = {
    more: $localize`:@@messageText.more:Show more`,
    less: $localize`:@@messageText.less:Show less`,
  } as const;
  protected readonly toggleText = computed(() =>
    this.expanded() ? this.copy.less : this.copy.more,
  );

  protected toggle(): void {
    this.expanded.update((expanded) => !expanded);
  }
}
