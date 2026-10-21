import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  LOCALE_ID,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import { GlzIconButtonComponent, GlzIconComponent, GlzNavItemComponent } from '@glave/glaze';
import { ListChecks, PenLine, Search } from 'lucide';

import { TasksService } from '../../services/tasks.service';
import { PersonComponent } from '../person/person.component';
import { ProjectsNavComponent } from '../projects-nav/projects-nav.component';

/**
 * The side nav: glave's name with New chat beside it, and the search; what needs the person, the
 * person's projects with their threads, then their chats; and at its foot Tasks (only while
 * tasks are going or a schedule will start one) and the person signed in, with their pages.
 */
@Component({
  selector: 'app-side-nav',
  imports: [
    RouterLink,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzNavItemComponent,
    PersonComponent,
    ProjectsNavComponent,
  ],
  templateUrl: './side-nav.component.html',
  styleUrl: './side-nav.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SideNavComponent {
  /** The thread shown now, to mark it current. */
  readonly current = input<string | null>(null);
  /** The page shown now (`tasks`, `memory`, `settings`), to mark Tasks current on its own. */
  readonly page = input<string | null>(null);
  /** Emitted when the person follows a link, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = {
    name: 'glave',
    newChat: $localize`:@@sideNav.newChat:New chat`,
    search: $localize`:@@sideNav.search:Search threads`,
    searchPlaceholder: $localize`:@@sideNav.searchPlaceholder:Search`,
    tasks: $localize`:@@sideNav.tasks:Tasks`,
  } as const;
  protected readonly icons = { newChat: PenLine, search: Search, tasks: ListChecks };
  protected readonly tasksLink = '/tasks';
  /** What the person searches the thread titles for. */
  protected readonly query = signal('');
  private readonly tasks = inject(TasksService);
  protected readonly tasksLive = this.tasks.live;
  private readonly locale = inject(LOCALE_ID);
  /**
   * How many tasks are going, as the reader writes numbers: shown alone, and read to screen
   * readers in words.
   */
  protected readonly tasksGoing = computed(() => {
    const going = this.tasks.going();
    const count = new Intl.NumberFormat(this.locale).format(going);
    return going === 0 ? null : { count, words: $localize`:@@sideNav.going:${count}:COUNT: going` };
  });
  protected readonly tasksCurrent = computed(() => this.page() === 'tasks');

  constructor() {
    const unwatch = this.tasks.watch();
    inject(DestroyRef).onDestroy(unwatch);
  }

  protected search(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
    }
  }

  protected followed(): void {
    this.navigated.emit();
  }
}
