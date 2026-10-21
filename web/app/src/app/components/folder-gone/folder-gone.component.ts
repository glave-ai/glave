import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { GlzButtonComponent, GlzConfirmDialogComponent, GlzDialogComponent } from '@glave/glaze';

import { projectProblem } from '../../lib/projects';
import { SessionService } from '../../services/session.service';
import { ProjectsService } from '../../services/projects.service';
import { FolderPickerComponent } from '../folder-picker/folder-picker.component';

/**
 * A project whose folder isn't there anymore, in the server's words, and what the person on
 * this computer can do about it: Find it picks the folder's new place with the folder browser,
 * and Remove the project, once confirmed, puts its threads in the archive. Anyone else reads
 * the words alone, since only the person on this computer reaches its folders. A refusal shows
 * under it, in the server's words.
 */
@Component({
  selector: 'app-folder-gone',
  imports: [
    GlzButtonComponent,
    GlzConfirmDialogComponent,
    GlzDialogComponent,
    FolderPickerComponent,
  ],
  templateUrl: './folder-gone.component.html',
  styleUrl: './folder-gone.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class FolderGoneComponent {
  /** The test hook; its parts get `<testId>-<part>`. */
  readonly testId = input.required<string>();
  /** The project, by its project's id. */
  readonly project = input.required<string>();
  /** What the server says of it. */
  readonly message = input.required<string>();

  protected readonly copy = {
    find: $localize`:@@folderGone.find:Find it`,
    remove: $localize`:@@folderGone.remove:Remove the project`,
    findHeading: $localize`:@@folderGone.findHeading:Where is the project’s folder now?`,
    removeHeading: $localize`:@@folderGone.removeHeading:Remove the project?`,
    removeMessage: $localize`:@@folderGone.removeMessage:Its threads go to the archive, with their history.`,
    removeConfirm: $localize`:@@folderGone.removeConfirm:Remove`,
  } as const;
  private readonly projects = inject(ProjectsService);
  /** Whether the person here may act: only they reach this computer's folders. */
  protected readonly local = inject(SessionService).local;
  protected readonly ids = computed(() => {
    const id = this.testId();
    return {
      message: `${id}-message`,
      find: `${id}-find`,
      remove: `${id}-remove`,
      problem: `${id}-problem`,
      picking: `${id}-picking`,
      confirm: `${id}-confirm`,
    };
  });
  protected readonly picking = signal(false);
  protected readonly confirming = signal(false);
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected find(): void {
    this.problem.set(null);
    this.picking.set(true);
  }

  protected askRemove(): void {
    this.problem.set(null);
    this.confirming.set(true);
  }

  protected async picked(path: string): Promise<void> {
    this.picking.set(false);
    await this.act(() => this.projects.findFolder(this.project(), path));
  }

  protected async decided(remove: boolean): Promise<void> {
    if (remove) {
      await this.act(() => this.projects.remove(this.project()));
    }
  }

  private async act(change: () => Promise<void>): Promise<void> {
    this.busy.set(true);
    try {
      await change();
    } catch (error: unknown) {
      this.problem.set(projectProblem(error));
    } finally {
      this.busy.set(false);
    }
  }
}
