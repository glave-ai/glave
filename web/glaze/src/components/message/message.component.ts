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
 * One message in a conversation, with no avatar and no name over it: the person's own as a
 * bubble at the end of the line, with room before it, as in a text conversation; a model's
 * reply as plain full-width text; a note from the app in small, muted text under its author.
 * Its content (text, reasoning, receipts, approvals and questions) follows in order. Its
 * actions (`glzMessageActions`, such as copy) sit under it, at the bubble's end or the reply's
 * start: shown on hover and on keyboard focus where there is a pointer, always on touch.
 * Every message is an article named by its author. While `streaming`, it is marked busy for
 * screen readers and shows a caret.
 *
 * ```html
 * <glz-message from="model" author="Reply" [streaming]="running()">
 *   <glz-reasoning>…</glz-reasoning>
 *   <p>{{ text() }}</p>
 *   <button glzIconButton glzMessageActions label="Copy" [glzCopy]="text()">…</button>
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
  /** Who wrote it, read as the article's name; shown only over a note. */
  readonly author = input.required<string>();
  readonly streaming = input(false, { transform: booleanAttribute });

  protected readonly busy = computed(() => (this.streaming() ? 'true' : null));
  protected readonly fromClass = computed(() => `from-${this.from()}`);
  /** Only a note says who wrote it: the person's messages and the replies need no name. */
  protected readonly showAuthor = computed(() => this.from() === 'note');
}
