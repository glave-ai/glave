import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  booleanAttribute,
  computed,
  input,
  model,
  output,
  viewChild,
} from '@angular/core';
import { ArrowUp, Square } from 'lucide';

import { injectId } from '../../tokens/ids';
import { isApple } from '../../lib/shortcuts';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { input: 'input', stop: 'stop', send: 'send' } as const;

/**
 * Where the person writes to the model. The text area grows with what they write (CSS
 * `field-sizing`, up to a limit); Enter sends and Shift+Enter starts a new line (Cmd or Ctrl
 * with Enter sends too), and `send` emits the draft, trimmed. While the model is answering
 * (`busy`), Send becomes Stop, and Escape stops it; the person can go on writing their next
 * message. Slots on the toolbar hold what shapes the turn (`[glzComposerStart]`) and a model
 * picker (`[glzComposerEnd]`).
 *
 * ```html
 * <glz-composer [(value)]="draft" [busy]="running()" (send)="start($event)" (stop)="cancel()">
 *   <glz-menu-select glzComposerEnd label="Model" [options]="models" [(value)]="model" />
 * </glz-composer>
 * ```
 */
@Component({
  selector: 'glz-composer',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './composer.component.html',
  styleUrl: './composer.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzComposerComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The draft. */
  readonly value = model('');
  /** Whether the model is answering, so Stop shows instead of Send. */
  readonly busy = input(false, { transform: booleanAttribute });
  readonly disabled = input(false, { transform: booleanAttribute });
  readonly placeholder = input('Write a message');
  readonly label = input('Message');
  readonly sendLabel = input('Send');
  readonly stopLabel = input('Stop');
  readonly hint = input('Enter to send, Shift+Enter for a new line');
  readonly send = output<string>();
  readonly stop = output();

  protected readonly hintId = injectId('glz-composer-hint');
  protected readonly sendIcon = ArrowUp;
  protected readonly stopIcon = Square;
  private readonly textarea = viewChild.required<ElementRef<HTMLTextAreaElement>>('textarea');

  protected readonly canSend = computed(
    () => !this.disabled() && !this.busy() && this.value().trim() !== '',
  );
  protected readonly sendDisabled = computed(() => !this.canSend());

  /** Sends the draft, if there is one, and clears it. */
  submit(): void {
    if (!this.canSend()) {
      return;
    }
    this.send.emit(this.value().trim());
    this.value.set('');
  }

  protected typed(event: Event): void {
    if (event.target instanceof HTMLTextAreaElement) {
      this.value.set(event.target.value);
    }
  }

  protected keydown(event: KeyboardEvent): void {
    if (event.key === 'Escape' && this.busy()) {
      event.preventDefault();
      this.stop.emit();
      return;
    }
    if (event.key !== 'Enter' || event.isComposing) {
      return;
    }
    const view = this.textarea().nativeElement.ownerDocument.defaultView;
    const mod = view !== null && isApple(view.navigator) ? event.metaKey : event.ctrlKey;
    if (!event.shiftKey || mod) {
      event.preventDefault();
      this.submit();
    }
  }

  protected stopped(): void {
    this.stop.emit();
  }
}
