import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  effect,
  inject,
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

import { ACCOUNT_NAME_LIMIT } from '../../lib/connections';
import { plural } from '../../lib/plural';

/** The longest title `thread/rename` takes, and the longest name a project is given. */
const TITLE_LIMIT = 200;

/** What is renamed: a thread, by its title, a project or an app's account, by its name. */
export type Renamed = 'thread' | 'project' | 'account';

/**
 * A dialog to give a thread a new title, or a project or an app's account a new name;
 * `renamed` emits it, trimmed.
 */
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
  /** The dialog's heading, where the page has its own, and a note under the field. */
  readonly heading = input<string | null>(null);
  readonly note = input<string | null>(null);
  /** Whether the dialog is open. */
  readonly open = model(false);
  readonly renamed = output<string>();

  private readonly locale = inject(LOCALE_ID);
  protected readonly copy = {
    cancel: $localize`:@@renameDialog.cancel:Cancel`,
    save: $localize`:@@renameDialog.save:Save`,
  } as const;
  private readonly namings: Readonly<
    Record<Renamed, { readonly heading: string; readonly label: string; readonly limit: number }>
  > = {
    thread: {
      heading: $localize`:@@renameDialog.threadHeading:Rename the chat`,
      label: $localize`:@@renameDialog.threadLabel:Title`,
      limit: TITLE_LIMIT,
    },
    project: {
      heading: $localize`:@@renameDialog.projectHeading:Rename the project`,
      label: $localize`:@@renameDialog.projectLabel:Name`,
      limit: TITLE_LIMIT,
    },
    account: {
      heading: $localize`:@@renameDialog.accountHeading:Rename the account`,
      label: $localize`:@@renameDialog.accountLabel:Name`,
      limit: ACCOUNT_NAME_LIMIT,
    },
  };
  protected readonly naming = computed(() => this.namings[this.kind()]);
  protected readonly headed = computed(() => this.heading() ?? this.naming().heading);
  protected readonly limit = computed(() => this.naming().limit);
  protected readonly hint = computed(() => {
    const limit = this.limit();
    return plural(
      $localize`:@@renameDialog.hint:{${limit}:LIMIT:, plural, one {Up to # character} other {Up to # characters}}`,
      this.locale,
    );
  });
  protected readonly draft = signal('');
  private readonly trimmed = computed(() => this.draft().trim());
  protected readonly invalid = computed(
    () => this.trimmed() === '' || this.trimmed().length > this.limit(),
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
