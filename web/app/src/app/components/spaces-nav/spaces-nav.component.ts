import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzBadgeComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzNavItemComponent,
  GlzNavListComponent,
  type GlzOption,
} from '@glave/glaze';
import { Ellipsis, FolderPlus, Plus } from 'lucide';

import {
  CHATS_LIST,
  type NavProject,
  type NavSort,
  PROJECTS_LIST,
  navSections,
} from '../../lib/nav';
import { NavListsService } from '../../services/nav-lists.service';
import { SpacesService } from '../../services/spaces.service';
import { FolderGoneComponent } from '../folder-gone/folder-gone.component';
import { NavListMenuComponent } from '../nav-list-menu/nav-list-menu.component';
import { NavThreadComponent } from '../nav-thread/nav-thread.component';
import { NewProjectComponent } from '../new-project/new-project.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';

const COPY = {
  threads: 'Projects and chats',
  chats: 'Chats',
  projects: 'Projects',
  newProject: 'New project',
  newChat: 'New chat',
  startProject: 'Start a project',
  noMatch: 'No threads match',
  newIn: 'New thread in',
  optionsFor: 'Options for',
  projectsOptions: 'Options for Projects',
  chatsOptions: 'Options for Chats',
  renameFailed: 'Could not rename the project:',
} as const;

/** What a project's menu does besides sorting its threads. */
const RENAME = 'rename';
const PROJECT_ACTIONS: readonly GlzOption[] = [{ value: RENAME, label: 'Rename' }];

/**
 * The person's threads in the side nav: each project with its threads, then their chats, by
 * recent activity unless they sort a list by name, and what waits for them first. Each heading
 * opens and closes its list; beside it, on hover and focus, its + starts a project, a thread
 * in the project or a chat, and its ⋯ sorts the list (and renames a project). A closed list
 * still shows, with a dot, that something in it needs the person. A thread another started sits
 * under it. A project whose folder isn't there anymore says so, in the server's words: a mark
 * on its heading, and above its threads, what can be done about it. How the person left the
 * lists is kept in this browser. A search keeps the threads
 * whose title matches. A list with nothing in it does not show: with no projects yet, one quiet
 * row offers to start one, as the + beside Projects does once there are some.
 */
@Component({
  selector: 'app-spaces-nav',
  imports: [
    RouterLink,
    FolderGoneComponent,
    GlzBadgeComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzNavItemComponent,
    GlzNavListComponent,
    NavListMenuComponent,
    NavThreadComponent,
    NewProjectComponent,
    RenameDialogComponent,
  ],
  templateUrl: './spaces-nav.component.html',
  styleUrl: './spaces-nav.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SpacesNavComponent {
  /** The thread shown now, to mark it current. */
  readonly current = input<string | null>(null);
  /** What to search the thread titles for. */
  readonly query = input('');
  /** Emitted when the person follows a link, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = COPY;
  protected readonly icons = { add: Plus, more: Ellipsis, start: FolderPlus };
  protected readonly newLink = '/';
  protected readonly lists = { projects: PROJECTS_LIST, chats: CHATS_LIST } as const;
  protected readonly projectActions = PROJECT_ACTIONS;
  private readonly spaces = inject(SpacesService);
  private readonly navLists = inject(NavListsService);

  private readonly sections = computed(() =>
    navSections(this.spaces.spaces(), {
      query: this.query(),
      current: this.current(),
      collapsed: this.navLists.collapsed(),
      byName: this.navLists.byName(),
    }),
  );
  protected readonly projects = computed(() => this.sections().projects);
  protected readonly projectList = computed(() =>
    this.projects().list.map((project) => ({
      ...project,
      newLabel: `${COPY.newIn} ${project.name}`,
      newQuery: { space: project.id },
      optionsLabel: `${COPY.optionsFor} ${project.name}`,
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
      this.spaces.loaded() &&
      this.spaces.error() === null &&
      this.query().trim() === '' &&
      !this.spaces.spaces().some(({ space }) => space.kind === 'project'),
  );
  protected readonly noMatch = computed(
    () => this.query().trim() !== '' && !this.hasChats() && !this.hasProjects(),
  );
  /** The project being renamed, and whether its dialog is open. */
  private readonly renaming = signal<Pick<NavProject, 'id' | 'name'> | null>(null);
  protected readonly renamingName = computed(() => this.renaming()?.name ?? '');
  protected readonly renameOpen = signal(false);
  /** Why the last rename failed, until the next one. */
  protected readonly renameError = signal<string | null>(null);

  protected setExpanded(list: string, expanded: boolean): void {
    this.navLists.setExpanded(list, expanded);
  }

  protected setSort(list: string, sort: NavSort): void {
    this.navLists.setSort(list, sort);
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
      await this.spaces.rename(project.id, name);
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
