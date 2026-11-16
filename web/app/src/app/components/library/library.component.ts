import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzButtonComponent,
  GlzConfirmDialogComponent,
  GlzDialogComponent,
  GlzEmptyStateComponent,
  GlzFigureComponent,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { Image } from 'lucide';

import {
  EVERY,
  type ShownItem,
  filtered,
  kindChoices,
  projectChoices,
  shown,
} from '../../lib/library';
import { hasProjects } from '../../lib/nav';
import { FilesService, drawn } from '../../services/files.service';
import { LibraryService } from '../../services/library.service';
import { ProjectsService } from '../../services/projects.service';
import { LibraryTileComponent } from '../library-tile/library-tile.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

/** What stands where the grid would be: nothing, an empty Library, or nothing the filters keep. */
type Empty = 'none' | 'empty' | 'unmatched';

/**
 * The Library page: what glave made for the person, newest first, as a grid, filtered by kind
 * and, when they have a project beside their Chats, by project; one item opened large, with
 * Download, Delete and Go to chat.
 */
@Component({
  selector: 'app-library',
  imports: [
    RouterLink,
    GlzButtonComponent,
    GlzConfirmDialogComponent,
    GlzDialogComponent,
    GlzEmptyStateComponent,
    GlzFigureComponent,
    GlzMenuSelectComponent,
    LibraryTileComponent,
    PageHeaderComponent,
  ],
  templateUrl: './library.component.html',
  styleUrl: './library.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LibraryComponent {
  protected readonly copy = {
    heading: $localize`:@@library.heading:Library`,
    kind: $localize`:@@library.kind:Kind`,
    project: $localize`:@@library.project:Project`,
    emptyHeading: $localize`:@@library.emptyHeading:Nothing here yet`,
    emptyText: $localize`:@@library.emptyText:Pictures glave makes for you will appear here. Try asking for one in a chat.`,
    unmatched: $localize`:@@library.unmatched:Nothing here matches these filters.`,
    delete: $localize`:@@library.delete:Delete`,
    goToChat: $localize`:@@library.goToChat:Go to chat`,
    deleteHeading: $localize`:@@library.deleteHeading:Delete this picture from your Library?`,
    deleteMessage: $localize`:@@library.deleteMessage:The chat it came from keeps its copy.`,
  } as const;
  protected readonly icon = Image;
  private readonly library = inject(LibraryService);
  private readonly projects = inject(ProjectsService);
  private readonly files = inject(FilesService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly problem = this.library.error;
  /** The kind and the project kept in view. */
  protected readonly kind = signal<string | null>(EVERY);
  protected readonly project = signal<string | null>(EVERY);
  protected readonly kinds = kindChoices();
  /** With only their Chats, the person filters by no project. */
  protected readonly byProject = computed(() => hasProjects(this.projects.projects()));
  protected readonly projectChoices = computed(() =>
    projectChoices(
      this.library.items(),
      this.projects.projects().map(({ project }) => project),
    ),
  );
  /** The items the filters keep, as the grid draws them. */
  protected readonly shown = computed(() => {
    const now = Date.now();
    const filter = {
      kind: this.kind() ?? EVERY,
      project: this.byProject() ? (this.project() ?? EVERY) : EVERY,
    };
    return filtered(this.library.items(), filter).map((item) =>
      shown(item, now, { locale: this.locale }),
    );
  });
  protected readonly hasItems = computed(() => this.library.items().length > 0);
  protected readonly empty = computed<Empty>(() => {
    if (!this.library.loaded() || this.shown().length > 0) {
      return 'none';
    }
    return this.hasItems() ? 'unmatched' : 'empty';
  });
  /** The item opened large, if one is. */
  protected readonly opened = signal<ShownItem | null>(null);
  protected readonly previewOpen = signal(false);
  protected readonly deleteOpen = signal(false);
  private readonly picture = drawn(() => {
    const item = this.opened()?.item;
    return item === undefined ? null : this.files.holdAsset(item.asset, item.media_type);
  });
  protected readonly url = this.picture.url;
  protected readonly failed = this.picture.failed;

  constructor() {
    void this.library.refresh();
  }

  protected open(item: ShownItem): void {
    this.opened.set(item);
    this.previewOpen.set(true);
  }

  protected askDelete(): void {
    this.deleteOpen.set(true);
  }

  protected async decided(confirmed: boolean): Promise<void> {
    const item = this.opened()?.item;
    if (!confirmed || item === undefined) {
      return;
    }
    this.previewOpen.set(false);
    this.opened.set(null);
    await this.library.remove(item.asset);
  }
}
