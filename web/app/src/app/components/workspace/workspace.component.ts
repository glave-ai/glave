import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  viewChild,
} from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router, RouterOutlet } from '@angular/router';
import { GlzSidenavLayoutComponent } from '@glave/glaze';
import { filter, map } from 'rxjs';

import { pageFromUrl, threadFromUrl } from '../../lib/threads';
import { NavService } from '../../services/nav.service';
import { SessionService } from '../../services/session.service';
import { SpacesService } from '../../services/spaces.service';
import { SideNavComponent } from '../side-nav/side-nav.component';

const COPY = {
  sideNav: 'Spaces and threads',
  skip: 'Skip to content',
  resize: 'Resize the side bar',
  hint: 'Drag or use the arrow keys to resize; Enter collapses. Show or hide:',
} as const;

/**
 * The signed-in frame, in three panes: the side nav (threads by space, the pages, the person),
 * the page, and, in a thread, its tools. Each page has its own top bar. The side nav's edge
 * resizes and collapses it, and ⌘B or Ctrl+B shows or hides it anywhere.
 */
@Component({
  selector: 'app-workspace',
  imports: [RouterOutlet, GlzSidenavLayoutComponent, SideNavComponent],
  templateUrl: './workspace.component.html',
  styleUrl: './workspace.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'keydown($event)' },
})
export class WorkspaceComponent {
  protected readonly copy = COPY;
  protected readonly contentLink = '#content';
  private readonly nav = inject(NavService);
  protected readonly navOpen = this.nav.open;
  protected readonly navWidth = this.nav.width;
  protected readonly shortcut = this.nav.ariaShortcut;
  protected readonly hint = `${COPY.hint} ${this.nav.shortcut}`;
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
  protected readonly currentPage = computed(() => pageFromUrl(this.url()));

  constructor() {
    void inject(SpacesService).refresh();
    // A refused sign-in (or signing out) ends the session: back to the sign-in page.
    const session = inject(SessionService);
    effect(() => {
      if (!session.signedIn()) {
        void this.router.navigateByUrl('/sign-in');
      }
    });
  }

  protected keydown(event: KeyboardEvent): void {
    this.nav.keydown(event);
  }

  protected navigated(): void {
    this.layout().navigated();
  }
}
