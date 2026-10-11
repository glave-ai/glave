import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzBadgeComponent,
  GlzButtonComponent,
  GlzIconComponent,
  GlzNavItemComponent,
  GlzNavListComponent,
} from '@glave/glaze';
import { Hash, Plus } from 'lucide';

import { threadMark, threadTitle } from '../../lib/threads';
import { SpacesService } from '../../services/spaces.service';

const COPY = {
  newChat: 'New chat',
  threads: 'Spaces and threads',
  empty: 'No threads yet',
} as const;

/** The side nav: New chat, then each space with its threads, marked running or waiting. */
@Component({
  selector: 'app-spaces-nav',
  imports: [
    RouterLink,
    GlzBadgeComponent,
    GlzButtonComponent,
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
  /** Emitted when the person follows a link, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = COPY;
  protected readonly icons = { thread: Hash, add: Plus };
  private readonly spaces = inject(SpacesService);

  protected readonly lists = computed(() =>
    this.spaces.spaces().map(({ space, threads }) => ({
      id: space.id,
      name: space.name,
      empty: threads.length === 0,
      threads: threads.map((thread) => ({
        id: thread.id,
        link: ['/t', thread.id],
        title: threadTitle(thread),
        mark: threadMark(thread),
        current: thread.id === this.current(),
      })),
    })),
  );

  protected followed(): void {
    this.navigated.emit();
  }
}
