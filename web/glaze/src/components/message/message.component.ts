import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

/** Who a message is from: the person, the model, or a note from the app itself. */
export type GlzMessageFrom = 'person' | 'model' | 'note';

/**
 * One message in a conversation, with no avatar: the person's own as a bubble at the end of
 * the line, with room before it, as in a text conversation; a model's reply as full-width text
 * under its author's name (with Hope's sun for Hope); a note from the app in small, muted
 * text. Its content (text, reasoning, receipts, approvals and questions) follows in order.
 * Every message is an article named by its author. While `streaming`, it is marked busy for
 * screen readers and shows a caret.
 *
 * ```html
 * <glz-message from="model" author="Hope" hope [streaming]="running()">
 *   <glz-reasoning>…</glz-reasoning>
 *   <p>{{ text() }}</p>
 * </glz-message>
 * ```
 */
@Component({
  selector: 'glz-message',
  templateUrl: './message.component.html',
  styleUrl: './message.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'article',
    '[attr.aria-label]': 'author()',
    '[attr.aria-busy]': 'busy()',
    '[class]': 'fromClass()',
  },
})
export class GlzMessageComponent {
  readonly from = input<GlzMessageFrom>('model');
  /** The name shown, and read, for who wrote it. */
  readonly author = input.required<string>();
  /** Shows Hope's sun beside the author's name. */
  readonly hope = input(false, { transform: booleanAttribute });
  readonly streaming = input(false, { transform: booleanAttribute });

  protected readonly busy = computed(() => (this.streaming() ? 'true' : null));
  protected readonly fromClass = computed(() => `from-${this.from()}`);
  /** The person's own messages need no name over them; replies and notes say who. */
  protected readonly showAuthor = computed(() => this.from() !== 'person');
}
