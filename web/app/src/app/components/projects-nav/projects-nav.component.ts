import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzNavItemComponent,
  GlzNavListComponent,
  type GlzOption,
} from '@glave/glaze';
import { Ellipsis, FolderPlus, Plus } from 'lucide';

import { CHATS_LIST, type NavProject, PROJECTS_LIST, navSections } from '../../lib/nav';
import { NavListsService } from '../../services/nav-lists.service';
import { ProjectsService } from '../../services/projects.service';
import { FolderGoneComponent } from '../folder-gone/folder-gone.component';
import { NavListMenuComponent } from '../nav-list-menu/nav-list-menu.component';
import { NavThreadComponent } from '../nav-thread/nav-thread.component';
import { NeedsYouComponent } from '../needs-you/needs-you.component';
import { NewProjectComponent } from '../new-project/new-project.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';

/** What a project's menu does. */
const RENAME = 'rename';

/**
 * The person's threads in the side nav: Needs you pinned first while something waits for them,
 * then each project with its threads, then their chats, each list newest first in one order,
 * whatever waits or runs. Each heading opens and closes its list; beside it, on hover and focus,
 * its + starts a project, a thread in the project or a chat, and a project's ⋯ renames it. A
 * closed list still shows, with the small mark a waiting thread has, that something in it needs
 * the person. A thread another started sits under it. A project whose folder isn't there
 * anymore says so, in the server's words: a mark on its heading in the warning tone, and above
 * its threads, what can be done about it. How the person left the lists is kept in this
 * browser. A search keeps the threads it finds by title or by day. A list with nothing in it
 * does not show: with no projects yet, one quiet row offers to start one, as the + beside
 * Projects does once there are some.
 */
@Component({
  selector: 'app-projects-nav',
  imports: [
    RouterLink,
    FolderGoneComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzNavItemComponent,
    GlzNavListComponent,
    NavListMenuComponent,
    NavThreadComponent,
    NeedsYouComponent,
    NewProjectComponent,
    RenameDialogComponent,
  ],
  templateUrl: './projects-nav.component.html',
  styleUrl: './projects-nav.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ProjectsNavComponent {
  /** The thread shown now, to mark it current. */
  readonly current = input<string | null>(null);
  /** What to search the thread titles for. */
  readonly query = input('');
  /** Emitted when the person follows a link, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = {
    threads: $localize`:@@projectsNav.threads:Threads`,
    chats: $localize`:@@projectsNav.chats:Chats`,
    projects: $localize`:@@projectsNav.projects:Projects`,
    newProject: $localize`:@@projectsNav.newProject:New project`,
    newChat: $localize`:@@projectsNav.newChat:New chat`,
    startProject: $localize`:@@projectsNav.startProject:Start a project`,
    noMatch: $localize`:@@projectsNav.noMatch:No threads match`,
  } as const;
  protected readonly icons = { add: Plus, more: Ellipsis, start: FolderPlus };
  protected readonly newLink = '/';
  protected readonly lists = { projects: PROJECTS_LIST, chats: CHATS_LIST } as const;
  protected readonly projectActions: readonly GlzOption[] = [
    { value: RENAME, label: $localize`:@@projectsNav.rename:Rename` },
  ];
  private readonly service = inject(ProjectsService);
  private readonly navLists = inject(NavListsService);
  /** The reader's locale, which the threads' times are written in and searched by. */
  private readonly locale = inject(LOCALE_ID);

  private readonly sections = computed(() =>
    navSections(this.service.projects(), {
      query: this.query(),
      current: this.current(),
      collapsed: this.navLists.collapsed(),
      now: this.service.readAt(),
      reader: { locale: this.locale },
    }),
  );
  protected readonly needsYou = computed(() => this.sections().needsYou);
  protected readonly projects = computed(() => this.sections().projects);
  protected readonly projectList = computed(() =>
    this.projects().list.map((project) => ({
      ...project,
      newLabel: $localize`:@@projectsNav.newIn:New thread in ${project.name}:PROJECT:`,
      newQuery: { project: project.id },
      optionsLabel: $localize`:@@projectsNav.optionsFor:Options for ${project.name}:PROJECT:`,
    })),
  );
  protected readonly hasProjects = computed(() => this.projectList().length > 0);
  protected readonly chats = computed(() => this.sections().chats);
  protected readonly hasChats = computed(() => this.chats().list.length > 0);
  /** Whether New project is open. */
  protected readonly newProject = signal(false);
  /** Once the lists are read, with no project in them and no search, a row offers one. */
  protected readonly offerProject = computed(
    () =>
      this.service.loaded() &&
      this.service.error() === null &&
      this.query().trim() === '' &&
      !this.service.projects().some(({ project }) => project.kind === 'project'),
  );
  protected readonly noMatch = computed(
    () => this.query().trim() !== '' && !this.hasChats() && !this.hasProjects(),
  );
  /** The project being renamed, and whether its dialog is open. */
  private readonly renaming = signal<Pick<NavProject, 'id' | 'name'> | null>(null);
  protected readonly renamingName = computed(() => this.renaming()?.name ?? '');
  protected readonly renameOpen = signal(false);
  /** Why the last rename failed, as the server says, until the next one. */
  private readonly renameError = signal<string | null>(null);
  protected readonly renameProblem = computed(() => {
    const error = this.renameError();
    return error === null
      ? null
      : $localize`:@@projectsNav.renameFailed:Could not rename the project: ${error}:ERROR:`;
  });

  protected setExpanded(list: string, expanded: boolean): void {
    this.navLists.setExpanded(list, expanded);
  }

  protected act(project: NavProject, action: string): void {
    if (action === RENAME) {
      this.renaming.set({ id: project.id, name: project.name });
      this.renameOpen.set(true);
    }
  }

  protected async rename(name: string): Promise<void> {
    const project = this.renaming();
    if (project === null || name === project.name) {
      return;
    }
    try {
      await this.service.rename(project.id, name);
      this.renameError.set(null);
    } catch (error: unknown) {
      this.renameError.set(error instanceof Error ? error.message : String(error));
    }
  }

  protected startProject(): void {
    this.newProject.set(true);
  }

  protected followed(): void {
    this.navigated.emit();
  }
}
