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
import { Folder, Plus } from 'lucide';

import { navSections } from '../../lib/nav';
import { SpacesService } from '../../services/spaces.service';

const COPY = {
  threads: 'Chats and projects',
  chats: 'Chats',
  projects: 'Projects',
  noMatch: 'No threads match',
  newIn: 'New thread in',
} as const;

/**
 * The person's threads in the side nav: their chats, then each project with its threads under
 * it. A project opens and closes from its row, and its + (shown on the row) starts a thread in
 * it; each thread is marked, quietly, while it runs or needs the person. A search keeps the
 * threads whose title matches. A list with nothing in it does not show.
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
  protected readonly icons = { add: Plus, project: Folder };
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

  protected followed(): void {
    this.navigated.emit();
  }
}
