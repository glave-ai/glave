import { ChangeDetectionStrategy, Component, computed, inject, viewChild } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzSidenavLayoutComponent,
  GlzTopBarComponent,
} from '@glave/glaze';
import { Brain, ListChecks, Menu, Settings } from 'lucide';
import { filter, map } from 'rxjs';

import { threadFromUrl } from '../../lib/threads';
import { SpacesService } from '../../services/spaces.service';
import { SpacesNavComponent } from '../spaces-nav/spaces-nav.component';
import { ThemePickerComponent } from '../theme-picker/theme-picker.component';

const COPY = {
  name: 'glave',
  menu: 'Show or hide spaces and threads',
  sideNav: 'Spaces and threads',
  pages: 'Pages',
  tasks: 'Tasks',
  memory: 'Memory',
  settings: 'Settings',
  skip: 'Skip to content',
} as const;

/** The signed-in frame: spaces and threads at the side, the top bar, then the page. */
@Component({
  selector: 'app-workspace',
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzSidenavLayoutComponent,
    GlzTopBarComponent,
    SpacesNavComponent,
    ThemePickerComponent,
  ],
  templateUrl: './workspace.component.html',
  styleUrl: './workspace.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class WorkspaceComponent {
  protected readonly copy = COPY;
  protected readonly icons = { menu: Menu, tasks: ListChecks, memory: Brain, settings: Settings };
  private readonly router = inject(Router);
  private readonly layout = viewChild.required(GlzSidenavLayoutComponent);
  private readonly url = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.router.url),
    ),
    { initialValue: this.router.url },
  );
  protected readonly currentThread = computed(() => threadFromUrl(this.url()));

  constructor() {
    void inject(SpacesService).refresh();
  }

  protected toggle(): void {
    this.layout().toggle();
  }

  protected navigated(): void {
    this.layout().navigated();
  }
}
