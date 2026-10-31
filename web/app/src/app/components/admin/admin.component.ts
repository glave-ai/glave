import { ChangeDetectionStrategy, Component, effect, inject, viewChild } from '@angular/core';
import { Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import {
  GlzIconComponent,
  GlzNavItemComponent,
  GlzNavListComponent,
  GlzSidenavLayoutComponent,
} from '@glave/glaze';
import ArrowLeft from 'lucide/dist/esm/icons/arrow-left.mjs';
import Palette from 'lucide/dist/esm/icons/palette.mjs';
import Users from 'lucide/dist/esm/icons/users.mjs';

import { NavService } from '../../services/nav.service';
import { SessionService } from '../../services/session.service';

/**
 * Admin, a room apart from the person's chats, for the organization's Admins and Owners: its
 * side nav leads back to the chats and to its pages (People), and its page fills the rest.
 * The side nav is a drawer on narrow screens, opened from the page's own header, and ⌘B or
 * Ctrl+B shows or hides it, as the workspace's.
 */
@Component({
  selector: 'app-admin',
  imports: [
    RouterLink,
    RouterLinkActive,
    RouterOutlet,
    GlzIconComponent,
    GlzNavItemComponent,
    GlzNavListComponent,
    GlzSidenavLayoutComponent,
  ],
  templateUrl: './admin.component.html',
  styleUrl: './admin.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'keydown($event)' },
})
export class AdminComponent {
  protected readonly copy = {
    sideNav: $localize`:@@admin.sideNav:Admin`,
    skip: $localize`:@@admin.skip:Skip to content`,
    resize: $localize`:@@admin.resize:Resize the side bar`,
    back: $localize`:@@admin.back:Back to chats`,
    heading: $localize`:@@admin.heading:Admin`,
    people: $localize`:@@admin.people:People`,
    brand: $localize`:@@admin.brand:Brand`,
  } as const;
  protected readonly icons = { back: ArrowLeft, people: Users, brand: Palette } as const;
  protected readonly contentLink = '#content';
  protected readonly chatsLink = '/';
  protected readonly peopleLink = '/admin';
  protected readonly brandLink = '/admin/brand';
  /** People is current on its own page alone: Brand's is under it. */
  protected readonly exact = { exact: true } as const;
  private readonly nav = inject(NavService);
  protected readonly navOpen = this.nav.open;
  protected readonly navWidth = this.nav.width;
  protected readonly shortcut = this.nav.ariaShortcut;
  protected readonly hint = $localize`:@@admin.hint:Drag or use the arrow keys to resize; Enter collapses. Show or hide: ${this.nav.shortcut}:SHORTCUT:`;
  private readonly layout = viewChild.required(GlzSidenavLayoutComponent);
  private readonly router = inject(Router);
  private readonly session = inject(SessionService);

  /** A refused sign-in (or signing out) ends the session: back to the sign-in page. */
  protected readonly leaving = effect(() => {
    if (!this.session.signedIn()) {
      void this.router.navigateByUrl('/sign-in');
    }
  });

  protected keydown(event: KeyboardEvent): void {
    this.nav.keydown(event);
  }

  protected navigated(): void {
    this.layout().navigated();
  }
}
