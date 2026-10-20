import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  booleanAttribute,
  computed,
  input,
  model,
  output,
  signal,
  viewChild,
} from '@angular/core';
import { ArrowUp, Plus, SlidersHorizontal, Square } from 'lucide';

import { injectId } from '../../tokens/ids';
import { isApple } from '../../lib/shortcuts';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  attach: 'attach',
  picker: 'picker',
  input: 'input',
  stop: 'stop',
  send: 'send',
  options: 'options',
  status: 'status',
} as const;

/** A composer's size: a reply's (`md`), or the larger one that starts a thread (`lg`). */
export type GlzComposerSize = 'md' | 'lg';

/**
 * Where the person writes to the model. The box holds only what they write: the text area,
 * which grows with what they write (CSS `field-sizing`, up to a limit), what sets where it
 * goes (`[glzComposerStart]`, such as a project chip), and send. Under the box, a quiet status
 * line (`[glzComposerStatus]`) holds what shapes the turn: the approval mode and the model,
 * small and muted, with no box of their own. On a phone (`GLZ_NARROW`) the composer is one row
 * (an options button, the text, send), and the options button opens the status line as a
 * sheet; what sits in the box's start slot is hidden there, so an app moves it to the status
 * line at that width.
 *
 * Enter sends and Shift+Enter starts a new line (Cmd or Ctrl with Enter sends too), and `send`
 * emits the draft, trimmed. While the model is answering (`busy`), Stop takes Send's place,
 * and Escape stops it; the person can go on writing, and sends with Enter, or with Send, which
 * comes back beside Stop once there is something to send, for the app to give the model while
 * it works. A `lg` composer starts a thread: two lines to write in, in larger type, more room
 * round them, rounder corners, and a shadow under it.
 *
 * With an `attachLabel`, a + at the box's start opens the system's file picker, and files
 * pasted into the box or dropped on the composer are added too: `attach` emits them, and the app
 * shows each as a chip in `[glzComposerAttachments]`, a row above the text. `attached` lets a
 * message with files and no text be sent; `waiting` holds Send while one is on its way.
 *
 * ```html
 * <glz-composer [(value)]="draft" [busy]="running()" (send)="start($event)" (stop)="cancel()">
 *   <glz-menu-select glzComposerStatus label="Model" quiet [options]="models" [(value)]="model" />
 * </glz-composer>
 * ```
 */
@Component({
  selector: 'glz-composer',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './composer.component.html',
  styleUrl: './composer.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.glz-composer--lg]': 'large()',
    '[class.glz-composer--attach]': 'attachable()',
    '[class.glz-composer--dragging]': 'dragging()',
    '(dragover)': 'dragover($event)',
    '(dragleave)': 'dragleave()',
    '(drop)': 'drop($event)',
  },
})
export class GlzComposerComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The draft. */
  readonly value = model('');
  /** Whether the model is answering, so Stop shows where Send was. */
  readonly busy = input(false, { transform: booleanAttribute });
  readonly disabled = input(false, { transform: booleanAttribute });
  readonly size = input<GlzComposerSize>('md');
  readonly placeholder = input('Write a message');
  readonly label = input('Message');
  readonly sendLabel = input('Send');
  readonly stopLabel = input('Stop');
  /** Names the button that opens the status line on a phone, and the line itself. */
  readonly optionsLabel = input('Options');
  readonly hint = input('Enter to send, Shift+Enter for a new line');
  /**
   * Names the button at the box's start that adds files; with it, files pasted into the box
   * or dropped on the composer are added too (`attach`). With none, files are not taken.
   */
  readonly attachLabel = input<string | null>(null);
  /** How many files go with the message: with one, it may be sent with no text. */
  readonly attached = input(0);
  /** Whether a file is still on its way: Send waits for it. */
  readonly waiting = input(false, { transform: booleanAttribute });
  readonly send = output<string>();
  readonly stop = output();
  /** Files the person added, by the button, pasting or dropping them. */
  readonly attach = output<readonly File[]>();

  protected readonly hintId = injectId('glz-composer-hint');
  protected readonly statusId = injectId('glz-composer-status');
  protected readonly large = computed(() => this.size() === 'lg');
  protected readonly icons = {
    send: ArrowUp,
    stop: Square,
    options: SlidersHorizontal,
    attach: Plus,
  };
  private readonly textarea = viewChild.required<ElementRef<HTMLTextAreaElement>>('textarea');
  private readonly picker = viewChild<ElementRef<HTMLInputElement>>('picker');

  protected readonly attachable = computed(() => this.attachLabel() !== null);
  /** Whether files are being dragged over the composer. */
  protected readonly dragging = signal(false);
  protected readonly canSend = computed(
    () =>
      !this.disabled() && !this.waiting() && (this.value().trim() !== '' || this.attached() > 0),
  );
  protected readonly sendDisabled = computed(() => !this.canSend());
  /** Whether Send shows beside Stop: while busy, once there is something to send. */
  protected readonly sendBesideStop = computed(() => this.busy() && this.canSend());

  /** Puts the cursor in the box, so the person can start writing. */
  focus(): void {
    this.textarea().nativeElement.focus();
  }

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

  /** Opens the system's file picker. */
  protected pick(): void {
    this.picker()?.nativeElement.click();
  }

  protected picked(event: Event): void {
    if (!(event.target instanceof HTMLInputElement)) {
      return;
    }
    this.added(event.target.files);
    // Picking the same file again adds it again.
    event.target.value = '';
  }

  /** A paste that holds files adds them; text pastes as ever. */
  protected pasted(event: ClipboardEvent): void {
    const files = event.clipboardData?.files;
    if (!this.attachable() || files === undefined || files.length === 0) {
      return;
    }
    event.preventDefault();
    this.added(files);
  }

  protected dragover(event: DragEvent): void {
    if (!this.attachable() || !carriesFiles(event)) {
      return;
    }
    event.preventDefault();
    this.dragging.set(true);
  }

  protected dragleave(): void {
    this.dragging.set(false);
  }

  protected drop(event: DragEvent): void {
    this.dragging.set(false);
    const files = event.dataTransfer?.files;
    if (!this.attachable() || files === undefined || files.length === 0) {
      return;
    }
    event.preventDefault();
    this.added(files);
  }

  private added(files: FileList | null): void {
    if (files !== null && files.length > 0) {
      this.attach.emit(Array.from(files));
    }
  }
}

/** Whether what is dragged holds files, rather than text or a link. */
function carriesFiles(event: DragEvent): boolean {
  return event.dataTransfer?.types.includes('Files') ?? false;
}
