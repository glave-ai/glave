import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import {
  GlzButtonComponent,
  GlzCheckboxComponent,
  GlzConfirmDialogComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';

const COPY = {
  heading: 'Dialogs',
  openDialog: 'Rename the thread',
  openConfirm: 'Delete the thread',
  dialogHeading: 'Rename the thread',
  title: 'Title',
  cancel: 'Cancel',
  save: 'Save',
  confirmHeading: 'Delete the thread?',
  confirmMessage: 'It is hidden for good, and erased with its keys.',
  delete: 'Delete',
  openLong: 'Keep threads',
  longHeading: 'Choose the threads to keep',
  done: 'Done',
} as const;

/** More choices than a screen holds, so the dialog's body scrolls (Tab scrolls it too). */
const CHOICES = Array.from({ length: 24 }, (_, index) => `Thread ${String(index + 1)}`);

/** The showcase's dialogs: a short one, a confirmation, and one whose body scrolls. */
@Component({
  selector: 'app-showcase-overlays',
  imports: [
    GlzButtonComponent,
    GlzCheckboxComponent,
    GlzConfirmDialogComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
  ],
  templateUrl: './showcase-overlays.component.html',
  styleUrl: './showcase-overlays.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseOverlaysComponent {
  protected readonly copy = COPY;
  protected readonly renaming = signal(false);
  protected readonly confirming = signal(false);
  protected readonly choosing = signal(false);
  protected readonly choices = CHOICES;

  protected openRename(): void {
    this.renaming.set(true);
  }

  protected openConfirm(): void {
    this.confirming.set(true);
  }

  protected closeRename(): void {
    this.renaming.set(false);
  }

  protected openLong(): void {
    this.choosing.set(true);
  }

  protected closeLong(): void {
    this.choosing.set(false);
  }
}
