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
import { ProjectsService } from '../../services/projects.service';
import { PasskeyOfferComponent } from '../passkey-offer/passkey-offer.component';
import { SideNavComponent } from '../side-nav/side-nav.component';

/**
 * The signed-in frame, in three panes: the side nav (threads by project, the pages, the person),
 * the page, and, in a thread, its tools. Each page has its own top bar. The side nav's edge
 * resizes and collapses it, and ⌘B or Ctrl+B shows or hides it anywhere. Signed in by glave's
 * link on this computer, it offers once to make a passkey.
 */
@Component({
  selector: 'app-workspace',
  imports: [RouterOutlet, GlzSidenavLayoutComponent, PasskeyOfferComponent, SideNavComponent],
  templateUrl: './workspace.component.html',
  styleUrl: './workspace.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'keydown($event)' },
})
export class WorkspaceComponent {
  protected readonly copy = {
    sideNav: $localize`:@@workspace.sideNav:Projects and threads`,
    skip: $localize`:@@workspace.skip:Skip to content`,
    resize: $localize`:@@workspace.resize:Resize the side bar`,
  } as const;
  protected readonly contentLink = '#content';
  private readonly nav = inject(NavService);
  protected readonly navOpen = this.nav.open;
  protected readonly navWidth = this.nav.width;
  protected readonly shortcut = this.nav.ariaShortcut;
  protected readonly hint = $localize`:@@workspace.hint:Drag or use the arrow keys to resize; Enter collapses. Show or hide: ${this.nav.shortcut}:SHORTCUT:`;
  private readonly router = inject(Router);
  private readonly session = inject(SessionService);
  private readonly layout = viewChild.required(GlzSidenavLayoutComponent);
  private readonly url = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.router.url),
    ),
    { initialValue: this.router.url },
  );
  /** Whether the offer to make a passkey here may show, which is loaded only then. */
  protected readonly offering = this.session.offersPasskey;
  protected readonly currentThread = computed(() => threadFromUrl(this.url()));
  protected readonly currentPage = computed(() => pageFromUrl(this.url()));

  /** A refused sign-in (or signing out) ends the session: back to the sign-in page. */
  protected readonly leaving = effect(() => {
    if (!this.session.signedIn()) {
      void this.router.navigateByUrl('/sign-in');
    }
  });

  constructor() {
    inject(ProjectsService).follow();
  }

  protected keydown(event: KeyboardEvent): void {
    this.nav.keydown(event);
  }

  protected navigated(): void {
    this.layout().navigated();
  }
}
