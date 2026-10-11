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
import { ArrowUp, Paperclip, Square, X } from 'lucide';

import { formatBytes } from '../../lib/format';
import { injectId } from '../../lib/ids';
import { isApple } from '../../lib/shortcuts';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';

/** What the person sends: the text, trimmed, and any files attached. */
export interface GlzComposed {
  readonly text: string;
  readonly files: readonly File[];
}

/** An attached file as the composer shows it. */
interface ShownFile {
  readonly index: number;
  readonly name: string;
  readonly size: string;
  readonly removeLabel: string;
}

/**
 * Where the person writes to the model. The text area grows with what they write (CSS
 * `field-sizing`, up to a limit); Enter sends and Shift+Enter starts a new line (Cmd or Ctrl
 * with Enter sends too); files can be attached by button, by drop or by paste. While the
 * model is answering (`busy`), Send becomes Stop, and Escape stops it; the person can go on
 * writing their next message. Slots on the toolbar hold a model picker
 * (`[glzComposerStart]`) and toggles such as "Run in the cloud" (`[glzComposerEnd]`).
 *
 * ```html
 * <glz-composer [(value)]="draft" [busy]="running()" (send)="start($event)" (stop)="cancel()">
 *   <glz-combobox glzComposerStart label="Model" [options]="models" [(value)]="model" />
 *   <glz-switch glzComposerEnd [(checked)]="cloud">Run in the cloud</glz-switch>
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
    '(dragover)': 'dragOver($event)',
    '(drop)': 'dropped($event)',
  },
})
export class GlzComposerComponent {
  /** The draft. */
  readonly value = model('');
  /** Files attached to the draft. */
  readonly files = model<readonly File[]>([]);
  /** Whether the model is answering, so Stop shows instead of Send. */
  readonly busy = input(false, { transform: booleanAttribute });
  readonly disabled = input(false, { transform: booleanAttribute });
  /** Whether files can be attached. */
  readonly attachments = input(true, { transform: booleanAttribute });
  readonly placeholder = input('Write a message');
  readonly label = input('Message');
  readonly sendLabel = input('Send');
  readonly stopLabel = input('Stop');
  readonly attachLabel = input('Attach files');
  readonly removeLabel = input('Remove');
  readonly hint = input('Enter to send, Shift+Enter for a new line');
  readonly send = output<GlzComposed>();
  readonly stop = output();

  protected readonly hintId = injectId('glz-composer-hint');
  protected readonly sendIcon = ArrowUp;
  protected readonly stopIcon = Square;
  protected readonly attachIcon = Paperclip;
  protected readonly removeIcon = X;
  private readonly fileInput = viewChild.required<ElementRef<HTMLInputElement>>('fileInput');
  private readonly textarea = viewChild.required<ElementRef<HTMLTextAreaElement>>('textarea');

  protected readonly shownFiles = computed<readonly ShownFile[]>(() =>
    this.files().map((file, index) => ({
      index,
      name: file.name,
      size: formatBytes(file.size),
      removeLabel: `${this.removeLabel()} ${file.name}`,
    })),
  );
  protected readonly hasFiles = computed(() => this.files().length > 0);
  protected readonly canSend = computed(
    () =>
      !this.disabled() && !this.busy() && (this.value().trim() !== '' || this.files().length > 0),
  );
  protected readonly sendDisabled = computed(() => !this.canSend());

  /** Sends the draft, if there is one, and clears it. */
  submit(): void {
    if (!this.canSend()) {
      return;
    }
    this.send.emit({ text: this.value().trim(), files: this.files() });
    this.value.set('');
    this.files.set([]);
  }

  /** Puts focus in the text area. */
  focus(): void {
    this.textarea().nativeElement.focus();
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

  protected chooseFiles(): void {
    this.fileInput().nativeElement.click();
  }

  protected picked(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.attach(event.target.files);
      event.target.value = '';
    }
  }

  protected pasted(event: ClipboardEvent): void {
    const files = event.clipboardData?.files;
    if (this.attachments() && files !== undefined && files.length > 0) {
      event.preventDefault();
      this.attach(files);
    }
  }

  protected dragOver(event: DragEvent): void {
    if (this.attachments() && event.dataTransfer?.types.includes('Files') === true) {
      event.preventDefault();
    }
  }

  protected dropped(event: DragEvent): void {
    const files = event.dataTransfer?.files;
    if (this.attachments() && files !== undefined && files.length > 0) {
      event.preventDefault();
      this.attach(files);
    }
  }

  protected remove(index: number): void {
    this.files.update((files) => files.filter((_, at) => at !== index));
  }

  private attach(list: FileList | null): void {
    if (list !== null) {
      const added = Array.from(list);
      this.files.update((files) => [...files, ...added]);
    }
  }
}
