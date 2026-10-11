import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

import { GlzAvatarComponent } from '../avatar/avatar.component';

/** Who a message is from: the person, the model, or a note from the app itself. */
export type GlzMessageFrom = 'person' | 'model' | 'note';

/**
 * One message in a conversation: who it is from, then its content (text, reasoning, receipts,
 * approvals and questions, in order). While `streaming`, it is marked busy for screen readers
 * and shows a caret.
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
  imports: [GlzAvatarComponent],
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
  /** Shows Hope's presence on the avatar. */
  readonly hope = input(false, { transform: booleanAttribute });
  readonly streaming = input(false, { transform: booleanAttribute });

  protected readonly busy = computed(() => (this.streaming() ? 'true' : null));
  protected readonly fromClass = computed(() => `from-${this.from()}`);
  protected readonly showAvatar = computed(() => this.from() !== 'note');
}
