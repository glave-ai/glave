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
import { Plus } from 'lucide';

import { navGroups } from '../../lib/nav';
import { SpacesService } from '../../services/spaces.service';

const COPY = {
  threads: 'Spaces and threads',
  empty: 'No threads yet',
  noMatch: 'No threads match',
  newIn: 'New thread in',
} as const;

/**
 * The spaces and their threads in the side nav: each project as a group the person can close,
 * with + to start a thread in it, then the loose chats; each thread marked running or
 * waiting. A search keeps the threads whose title matches.
 */
@Component({
  selector: 'app-spaces-nav',
  imports: [
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
  protected readonly addIcon = Plus;
  protected readonly newLink = '/';
  private readonly spaces = inject(SpacesService);
  /** The groups the person closed. */
  private readonly collapsed = signal<ReadonlySet<string>>(new Set());

  protected readonly searching = computed(() => this.query().trim() !== '');
  protected readonly groups = computed(() =>
    navGroups(this.spaces.spaces(), {
      query: this.query(),
      current: this.current(),
      collapsed: this.collapsed(),
    }).map((group) => ({
      ...group,
      newLabel: `${COPY.newIn} ${group.name}`,
      newQuery: { space: group.space },
    })),
  );

  protected setExpanded(group: string, expanded: boolean): void {
    this.collapsed.update((collapsed) => {
      const next = new Set(collapsed);
      if (expanded) {
        next.delete(group);
      } else {
        next.add(group);
      }
      return next;
    });
  }

  protected followed(): void {
    this.navigated.emit();
  }
}
