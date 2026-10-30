import { ChangeDetectionStrategy, Component, inject, output, signal } from '@angular/core';
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
import UserPlus from 'lucide/dist/esm/icons/user-plus.mjs';

import { ServerService } from '../../services/server.service';
import { InviteDialogComponent } from '../invite-dialog/invite-dialog.component';
import { UnloadedComponent } from '../unloaded/unloaded.component';
import { SessionService } from '../../services/session.service';

/**
 * The person signed in, at the foot of the side nav: opened, their Settings, Admin for the
 * organization's Admins and Owners where the server shows one, else "Invite someone" where
 * they may invite though they are alone, and Sign out. The session API does not say who
 * someone is yet, so it says "You".
 */
@Component({
  selector: 'app-person',
  imports: [
    InviteDialogComponent,
    UnloadedComponent,
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
    invite: $localize`:@@person.invite:Invite someone`,
    signOut: $localize`:@@person.signOut:Sign out`,
  } as const;
  protected readonly icons = {
    settings: Settings,
    admin: ShieldUser,
    invite: UserPlus,
    signOut: LogOut,
  };
  protected readonly settingsLink = '/settings';
  protected readonly adminLink = '/admin';
  /** Whether Admin is in the menu: the server says the person is an admin of one it shows. */
  private readonly server = inject(ServerService);
  protected readonly admin = this.server.admin;
  /** Whether "Invite someone" is in the menu: alone, the person may still make the first. */
  protected readonly invitesAlone = this.server.invitesAlone;
  /** Whether the dialog that makes an invite is open; its code loads the first time it is. */
  protected readonly inviting = signal(false);
  private readonly session = inject(SessionService);

  constructor() {
    // What the menu offers is the server's to say, whichever page opened first.
    void this.server.load();
  }

  protected followed(): void {
    this.navigated.emit();
  }

  protected invite(): void {
    this.inviting.set(true);
  }

  protected signOut(): void {
    void this.session.signOut();
  }
}
