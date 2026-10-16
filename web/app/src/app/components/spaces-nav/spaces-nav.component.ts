import { NgTemplateOutlet } from '@angular/common';
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
} from '@glave/glaze';
import { Folder, FolderPlus, Plus } from 'lucide';

import { navSections } from '../../lib/nav';
import { SpacesService } from '../../services/spaces.service';
import { NewProjectComponent } from '../new-project/new-project.component';

const COPY = {
  threads: 'Chats and projects',
  chats: 'Chats',
  projects: 'Projects',
  newProject: 'New project',
  startProject: 'Start a project',
  noMatch: 'No threads match',
  newIn: 'New thread in',
} as const;

/**
 * The person's threads in the side nav: their chats, then each project with its threads under
 * it. A project opens and closes from its row, and its + (shown on the row) starts a thread in
 * it; each thread is marked, quietly, while it runs or needs the person. A search keeps the
 * threads whose title matches. A list with nothing in it does not show: with no projects yet,
 * one quiet row offers to start one, as the + beside Projects does once there are some.
 */
@Component({
  selector: 'app-spaces-nav',
  imports: [
    NgTemplateOutlet,
    RouterLink,
    GlzBadgeComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzNavItemComponent,
    GlzNavListComponent,
    NewProjectComponent,
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
  protected readonly icons = { add: Plus, project: Folder, start: FolderPlus };
  protected readonly newLink = '/';
  private readonly spaces = inject(SpacesService);
  /** The projects the person closed. */
  private readonly collapsed = signal<ReadonlySet<string>>(new Set());

  private readonly sections = computed(() =>
    navSections(this.spaces.spaces(), {
      query: this.query(),
      current: this.current(),
      collapsed: this.collapsed(),
    }),
  );
  protected readonly chats = computed(() => this.sections().chats);
  protected readonly hasChats = computed(() => this.chats().length > 0);
  protected readonly projects = computed(() =>
    this.sections().projects.map((project) => ({
      ...project,
      newLabel: `${COPY.newIn} ${project.name}`,
      newQuery: { space: project.id },
    })),
  );
  protected readonly hasProjects = computed(() => this.projects().length > 0);
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

  protected setExpanded(project: string, expanded: boolean): void {
    this.collapsed.update((collapsed) => {
      const next = new Set(collapsed);
      if (expanded) {
        next.delete(project);
      } else {
        next.add(project);
      }
      return next;
    });
  }

  protected startProject(): void {
    this.newProject.set(true);
  }

  protected followed(): void {
    this.navigated.emit();
  }
}
