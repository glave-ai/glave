import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import {
  GlzButtonComponent,
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
} as const;

/** The showcase's dialog and confirmation. */
@Component({
  selector: 'app-showcase-overlays',
  imports: [
    GlzButtonComponent,
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

  protected openRename(): void {
    this.renaming.set(true);
  }

  protected openConfirm(): void {
    this.confirming.set(true);
  }

  protected closeRename(): void {
    this.renaming.set(false);
  }
}
