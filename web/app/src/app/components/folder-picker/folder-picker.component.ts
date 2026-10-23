import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  LOCALE_ID,
  afterNextRender,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
  viewChild,
  viewChildren,
} from '@angular/core';
import type { FolderListResult } from '@glave/client';
import {
  GlzButtonComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzNavItemComponent,
} from '@glave/glaze';
import { ChevronRight, Folder, HardDrive } from 'lucide';

import { crumbs, folderCount, folderName, joinPath, projectProblem } from '../../lib/projects';
import { SessionService } from '../../services/session.service';

/**
 * Picks a folder on the computer glave runs on, through `folder/list`. It opens at `start`, or
 * the home folder; shows where it is as the folders above it, each a button to go up to; and
 * lists the folders inside, each a button to go into. Use this folder picks the one shown,
 * unless the server says it is no project's folder (the home folder, one above it, the
 * temporary folder), and why, in its words; the folders inside may still be picked. It shows
 * folders' names alone, as the server gives them.
 */
@Component({
  selector: 'app-folder-picker',
  imports: [GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent, GlzNavItemComponent],
  templateUrl: './folder-picker.component.html',
  styleUrl: './folder-picker.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class FolderPickerComponent {
  /** The folder to open at, or `null` for the home folder. */
  readonly start = input<string | null>(null);
  /** Emits the folder picked, by its full path. */
  readonly picked = output<string>();

  protected readonly copy = {
    path: $localize`:@@folderPicker.path:Where you are`,
    top: $localize`:@@folderPicker.top:This computer`,
    none: $localize`:@@folderPicker.none:No folders in here.`,
    use: $localize`:@@folderPicker.use:Use this folder`,
  } as const;
  protected readonly icons = { top: HardDrive, folder: Folder, go: ChevronRight };
  private readonly session = inject(SessionService);
  private readonly locale = inject(LOCALE_ID);
  private readonly announcer = inject(LiveAnnouncer);
  private readonly injector = inject(Injector);
  private readonly folderButtons = viewChildren('folderButton', { read: ElementRef });
  private readonly useButton = viewChild.required('useButton', { read: ElementRef });
  /** The folder shown and what is in it. */
  private readonly listing = signal<FolderListResult | null>(null);
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  /** Which listing asked for last: an answer to an earlier one is dropped. */
  private asked = 0;

  /** Where it is, from the top down: the top as an icon, the folder shown marked current. */
  protected readonly crumbs = computed(() => {
    const listing = this.listing();
    const all = listing === null ? [] : crumbs(listing.path);
    return all.map((crumb, index) => ({
      ...crumb,
      top: crumb.name === '',
      after: index > 0,
      current: index === all.length - 1 ? 'location' : null,
    }));
  });
  protected readonly folders = computed(() => {
    const listing = this.listing();
    return (listing?.folders ?? []).map((name) => ({
      name,
      path: joinPath(listing?.path ?? '', name),
    }));
  });
  protected readonly listLabel = computed(() => {
    const folder = this.shownName(this.listing()?.path ?? '');
    return $localize`:@@folderPicker.foldersIn:Folders in ${folder}:FOLDER:`;
  });
  /** Why the folder shown is no project's folder, when the server says it is none. */
  protected readonly refused = computed(() => this.listing()?.refused ?? null);
  protected readonly cannotUse = computed(
    () => this.listing() === null || this.busy() || this.refused() !== null,
  );

  constructor() {
    effect(() => {
      const start = this.start();
      untracked(() => {
        void this.show(start, false);
      });
    });
  }

  /** Goes up to the folder at `path`; focus stays on the crumb pressed. */
  protected up(path: string): void {
    void this.show(path, false);
  }

  /** Goes into the folder at `path`; focus moves to what is in it. */
  protected into(path: string): void {
    void this.show(path, true);
  }

  protected use(): void {
    const listing = this.listing();
    if (listing !== null) {
      this.picked.emit(listing.path);
    }
  }

  /**
   * Lists the folder at `path` (`null`: the home folder). After moving, it says where it is
   * and, going into a folder whose button is now gone, moves focus to the first folder in it.
   * A folder that cannot be listed leaves the one shown, and says why.
   */
  private async show(path: string | null, focusIn: boolean): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    const moving = this.listing() !== null;
    this.busy.set(true);
    try {
      const listing = await this.session.call('folder/list', path === null ? {} : { path });
      if (asked !== this.asked) {
        return;
      }
      this.listing.set(listing);
      this.problem.set(null);
      if (moving) {
        const folder = this.shownName(listing.path);
        const count = folderCount(listing.folders.length, this.locale);
        void this.announcer.announce(
          $localize`:@@folderPicker.moved:${folder}:FOLDER:, ${count}:FOLDERS:`,
        );
      }
      if (focusIn) {
        afterNextRender(() => this.focusIn(), { injector: this.injector });
      }
    } catch (error: unknown) {
      if (asked === this.asked) {
        this.problem.set(projectProblem(error));
      }
    } finally {
      if (asked === this.asked) {
        this.busy.set(false);
      }
    }
  }

  private focusIn(): void {
    const first = this.folderButtons().at(0) ?? this.useButton();
    if (first.nativeElement instanceof HTMLElement) {
      first.nativeElement.focus();
    }
  }

  /** A folder's name as the picker says it: the top of the computer has none of its own. */
  private shownName(path: string): string {
    const name = folderName(path);
    return name === '' ? this.copy.top : name;
  }
}
