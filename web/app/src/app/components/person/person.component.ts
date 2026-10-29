import { ChangeDetectionStrategy, Component, inject, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzAvatarComponent,
  GlzNavItemComponent,
  GlzPopoverComponent,
  GlzPopoverTriggerDirective,
} from '@glave/glaze';
import LogOut from 'lucide/dist/esm/icons/log-out.mjs';
import Settings from 'lucide/dist/esm/icons/settings.mjs';
import ShieldUser from 'lucide/dist/esm/icons/shield-user.mjs';

import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';

/**
 * The person signed in, at the foot of the side nav: opened, their Settings, Admin for the
 * organization's Admins and Owners where the server shows one, and Sign out. The session API
 * does not say who someone is yet, so it says "You".
 */
@Component({
  selector: 'app-person',
  imports: [
    RouterLink,
    GlzAvatarComponent,
    GlzNavItemComponent,
    GlzPopoverComponent,
    GlzPopoverTriggerDirective,
  ],
  templateUrl: './person.component.html',
  styleUrl: './person.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PersonComponent {
  /** Emitted when the person goes to a page, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = {
    you: $localize`:@@person.you:You`,
    settings: $localize`:@@person.settings:Settings`,
    admin: $localize`:@@person.admin:Admin`,
    signOut: $localize`:@@person.signOut:Sign out`,
  } as const;
  protected readonly icons = { settings: Settings, admin: ShieldUser, signOut: LogOut };
  protected readonly settingsLink = '/settings';
  protected readonly adminLink = '/admin';
  /** Whether Admin is in the menu: the server says the person is an admin of one it shows. */
  protected readonly admin = inject(ServerService).admin;
  private readonly session = inject(SessionService);

  protected followed(): void {
    this.navigated.emit();
  }

  protected signOut(): void {
    void this.session.signOut();
  }
}
