import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzButtonComponent,
  GlzIconComponent,
  type GlzIconData,
  GlzInputDirective,
  GlzNavItemComponent,
  GlzNavListComponent,
} from '@glave/glaze';
import { Brain, ListChecks, Plus, Settings } from 'lucide';

import { InstallComponent } from '../install/install.component';
import { PersonComponent } from '../person/person.component';
import { SpacesNavComponent } from '../spaces-nav/spaces-nav.component';

const COPY = {
  newChat: 'New chat',
  search: 'Search threads',
  pages: 'Pages',
} as const;

/** A page the side nav links to below the threads. */
interface Page {
  readonly path: string;
  readonly label: string;
  readonly icon: GlzIconData;
  readonly testId: string;
}

const PAGES: readonly Page[] = [
  { path: 'tasks', label: 'Tasks', icon: ListChecks, testId: 'nav-tasks' },
  { path: 'memory', label: 'Memory', icon: Brain, testId: 'nav-memory' },
  { path: 'settings', label: 'Settings', icon: Settings, testId: 'nav-settings' },
];

/**
 * The side nav: New chat and the search at the top; the spaces with their threads; the Tasks,
 * Memory and Settings pages; and, at the foot, Install app (where the browser can) and the
 * person signed in.
 */
@Component({
  selector: 'app-side-nav',
  imports: [
    RouterLink,
    GlzButtonComponent,
    GlzIconComponent,
    GlzInputDirective,
    GlzNavItemComponent,
    GlzNavListComponent,
    InstallComponent,
    PersonComponent,
    SpacesNavComponent,
  ],
  templateUrl: './side-nav.component.html',
  styleUrl: './side-nav.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SideNavComponent {
  /** The thread shown now, to mark it current. */
  readonly current = input<string | null>(null);
  /** The page shown now (`tasks`, `memory`, `settings`), to mark it current. */
  readonly page = input<string | null>(null);
  /** Emitted when the person follows a link, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = COPY;
  protected readonly addIcon = Plus;
  /** What the person searches the thread titles for. */
  protected readonly query = signal('');
  protected readonly pages = computed(() =>
    PAGES.map((page) => ({ ...page, link: `/${page.path}`, current: page.path === this.page() })),
  );

  protected search(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
    }
  }

  protected followed(): void {
    this.navigated.emit();
  }
}
