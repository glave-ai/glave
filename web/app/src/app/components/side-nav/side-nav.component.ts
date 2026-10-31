import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  Injector,
  LOCALE_ID,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import { GlzIconButtonComponent, GlzIconComponent, GlzNavItemComponent } from '@glave/glaze';
import ListChecks from 'lucide/dist/esm/icons/list-checks.mjs';
import PenLine from 'lucide/dist/esm/icons/pen-line.mjs';
import Search from 'lucide/dist/esm/icons/search.mjs';

import { hasProjects } from '../../lib/nav';
import { GroupingService } from '../../services/grouping.service';
import { AppearanceService } from '../../services/appearance.service';
import type { BrandService } from '../../services/brand.service';
import { OrganizationsService } from '../../services/organizations.service';
import { ProjectsService } from '../../services/projects.service';
import { TasksService } from '../../services/tasks.service';
import { GroupByComponent } from '../group-by/group-by.component';
import { PersonComponent } from '../person/person.component';
import { ProjectsNavComponent } from '../projects-nav/projects-nav.component';
import { StatusNavComponent } from '../status-nav/status-nav.component';
import { UnloadedComponent } from '../unloaded/unloaded.component';

/**
 * The side nav: glave's name with New chat beside it, and the search; the person's threads,
 * grouped as they choose, by project (their projects, then their chats) or by what each is
 * doing, the switch above them while they have a project beside their Chats; and at its foot
 * Tasks (only while tasks are going or a schedule will start one) and the person signed in,
 * with their pages.
 */
@Component({
  selector: 'app-side-nav',
  imports: [
    RouterLink,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzNavItemComponent,
    GroupByComponent,
    PersonComponent,
    ProjectsNavComponent,
    StatusNavComponent,
    UnloadedComponent,
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
  private readonly projects = inject(ProjectsService);
  private readonly grouped = inject(GroupingService);
  private readonly organizations = inject(OrganizationsService);
  private readonly injector = inject(Injector);
  /** The organization's look, where it is shown to the person. */
  private readonly look = inject(AppearanceService).organization;
  /** What reads the organization's logo, loaded only for one that has a logo: few do. */
  private readonly brand = signal<BrandService | null>(null);
  /** What heads the side nav: the organization's name, the team the person works in, else glave. */
  protected readonly heading = computed(
    () => this.look()?.name ?? this.organizations.heading() ?? this.copy.name,
  );
  /** The organization's logo, in its name's place. */
  protected readonly logo = computed(() => this.brand()?.logoUrl() ?? null);
  protected readonly logoName = computed(
    () => this.look()?.name ?? $localize`:@@sideNav.logo:Your organization’s logo`,
  );
  private readonly grouping = this.grouped.grouping;
  /** Whether the lists have been read, so the threads come in all at once, in their place. */
  protected readonly read = this.projects.loaded;
  /** With only Chats there is nothing to group by project: no switch, and no projects named. */
  protected readonly groupable = computed(() => hasProjects(this.projects.projects()));
  protected readonly byStatus = computed(() => this.groupable() && this.grouping() === 'status');
  protected readonly byProject = computed(() => !this.byStatus());
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
    effect(() => {
      if ((this.look()?.logo ?? null) !== null && untracked(this.brand) === null) {
        void this.loadBrand();
      }
    });
    void this.organizations.load();
    const unwatch = this.tasks.watch();
    inject(DestroyRef).onDestroy(unwatch);
    // Read as the side nav opens, beside the lists' first read, so it groups them as it shows.
    void this.grouped.read();
  }

  private async loadBrand(): Promise<void> {
    try {
      const { BrandService } = await import('../../services/brand.service');
      this.brand.set(this.injector.get(BrandService));
    } catch {
      // It could not load: the name stands in the logo's place.
    }
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
