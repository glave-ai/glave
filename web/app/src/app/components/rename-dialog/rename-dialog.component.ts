import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  input,
  model,
  output,
  signal,
  untracked,
} from '@angular/core';
import {
  GlzButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';

/** The longest title `thread/rename` takes, and the longest name a project is given. */
const TITLE_LIMIT = 200;

/** What is renamed: a thread, by its title, or a project, by its name. */
export type Renamed = 'thread' | 'project';

const COPY = {
  hint: `Up to ${String(TITLE_LIMIT)} characters`,
  cancel: 'Cancel',
  save: 'Save',
} as const;

const NAMING: Readonly<Record<Renamed, { readonly heading: string; readonly label: string }>> = {
  thread: { heading: 'Rename the thread', label: 'Title' },
  project: { heading: 'Rename the project', label: 'Name' },
};

/** A dialog to give a thread a new title, or a project a new name; `renamed` emits it, trimmed. */
@Component({
  selector: 'app-rename-dialog',
  imports: [GlzButtonComponent, GlzDialogComponent, GlzFormFieldComponent, GlzInputDirective],
  templateUrl: './rename-dialog.component.html',
  styleUrl: './rename-dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RenameDialogComponent {
  /** What is renamed. */
  readonly kind = input<Renamed>('thread');
  /** Its title or name now, which the field starts with. */
  readonly title = input('');
  /** Whether the dialog is open. */
  readonly open = model(false);
  readonly renamed = output<string>();

  protected readonly copy = COPY;
  protected readonly naming = computed(() => NAMING[this.kind()]);
  protected readonly limit = TITLE_LIMIT;
  protected readonly draft = signal('');
  private readonly trimmed = computed(() => this.draft().trim());
  protected readonly invalid = computed(
    () => this.trimmed() === '' || this.trimmed().length > TITLE_LIMIT,
  );

  constructor() {
    // Each time it opens, the field starts from the title as it is.
    effect(() => {
      if (this.open()) {
        this.draft.set(untracked(this.title));
      }
    });
  }

  protected typed(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.draft.set(event.target.value);
    }
  }

  protected save(event?: Event): void {
    event?.preventDefault();
    if (this.invalid()) {
      return;
    }
    this.renamed.emit(this.trimmed());
    this.open.set(false);
  }

  protected cancel(): void {
    this.open.set(false);
  }
}
