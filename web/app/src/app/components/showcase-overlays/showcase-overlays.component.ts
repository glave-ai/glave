import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import {
  GlzButtonComponent,
  GlzConfirmDialogComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzInputDirective,
  GlzMenuComponent,
  GlzMenuItemComponent,
  GlzMenuTriggerDirective,
  GlzToastService,
} from '@glave/glaze';
import { Archive, EllipsisVertical, Pencil, Trash2 } from 'lucide';

const COPY = {
  heading: 'Dialogs, menus and toasts',
  dialogs: 'Dialogs and drawers',
  openDialog: 'Rename the thread',
  openDrawer: 'Open a drawer',
  openConfirm: 'Delete the thread',
  dialogHeading: 'Rename the thread',
  title: 'Title',
  cancel: 'Cancel',
  save: 'Save',
  drawerHeading: 'Thread details',
  drawerText: 'A drawer is a dialog along one side of the screen.',
  confirmHeading: 'Delete the thread?',
  confirmMessage: 'It is hidden for good, and erased with its keys.',
  delete: 'Delete',
  deleted: 'Thread deleted',
  kept: 'Kept',
  renamed: 'Renamed',
  menus: 'Menus',
  actions: 'Thread actions',
  rename: 'Rename',
  archive: 'Archive',
  archived: 'Archived',
  undo: 'Undo',
  restored: 'Back again',
  toasts: 'Toasts',
  info: 'Show a message',
  success: 'Show a success',
  danger: 'Show a failure',
  infoText: 'The server restarted.',
  successText: 'Saved.',
  dangerText: "Couldn't reach the server.",
} as const;

/** The showcase's dialog, drawer, confirmation, menu and toasts. */
@Component({
  selector: 'app-showcase-overlays',
  imports: [
    GlzButtonComponent,
    GlzConfirmDialogComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzInputDirective,
    GlzMenuComponent,
    GlzMenuItemComponent,
    GlzMenuTriggerDirective,
  ],
  templateUrl: './showcase-overlays.component.html',
  styleUrl: './showcase-overlays.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseOverlaysComponent {
  protected readonly copy = COPY;
  protected readonly icons = {
    more: EllipsisVertical,
    rename: Pencil,
    archive: Archive,
    remove: Trash2,
  };
  protected readonly renaming = signal(false);
  protected readonly drawer = signal(false);
  protected readonly confirming = signal(false);
  private readonly toasts = inject(GlzToastService);

  protected openRename(): void {
    this.renaming.set(true);
  }

  protected openDrawer(): void {
    this.drawer.set(true);
  }

  protected openConfirm(): void {
    this.confirming.set(true);
  }

  protected closeRename(): void {
    this.renaming.set(false);
  }

  protected saveRename(): void {
    this.renaming.set(false);
    this.toasts.show(COPY.renamed, { tone: 'success' });
  }

  protected decided(yes: boolean): void {
    this.toasts.show(yes ? COPY.deleted : COPY.kept);
  }

  protected archive(): void {
    this.toasts.show(COPY.archived, {
      action: { label: COPY.undo, run: () => this.toasts.show(COPY.restored) },
    });
  }

  protected info(): void {
    this.toasts.show(COPY.infoText);
  }

  protected success(): void {
    this.toasts.show(COPY.successText, { tone: 'success' });
  }

  protected failure(): void {
    this.toasts.show(COPY.dangerText, { tone: 'danger' });
  }
}
