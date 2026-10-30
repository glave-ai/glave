import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  output,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzAvatarComponent,
  GlzIconComponent,
  GlzNavItemComponent,
  GlzPopoverComponent,
  GlzPopoverTriggerDirective,
} from '@glave/glaze';
import LogOut from 'lucide/dist/esm/icons/log-out.mjs';
import Settings from 'lucide/dist/esm/icons/settings.mjs';
import ShieldUser from 'lucide/dist/esm/icons/shield-user.mjs';
import UserPlus from 'lucide/dist/esm/icons/user-plus.mjs';
import Check from 'lucide/dist/esm/icons/check.mjs';

import { type Place, place } from '../../lib/organizations';
import { OrganizationsService } from '../../services/organizations.service';
import { ServerService } from '../../services/server.service';
import { InviteDialogComponent } from '../invite-dialog/invite-dialog.component';
import { UnloadedComponent } from '../unloaded/unloaded.component';
import { SessionService } from '../../services/session.service';

/**
 * The person signed in, at the foot of the side nav: opened, where they are in more than one
 * organization the ones to switch to (their own as Personal, then each team, the current one
 * checked), their Settings, Admin for the
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
    GlzIconComponent,
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
    switchTo: $localize`:@@person.switchTo:Switch to`,
    signOut: $localize`:@@person.signOut:Sign out`,
  } as const;
  protected readonly icons = {
    settings: Settings,
    admin: ShieldUser,
    invite: UserPlus,
    current: Check,
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
  private readonly organizations = inject(OrganizationsService);
  private readonly locale = inject(LOCALE_ID);
  /** Whether the menu switches between organizations: the person is in more than one. */
  protected readonly several = this.organizations.several;
  /** The person's organizations, their own first, as the menu lists them. */
  protected readonly places = computed(() =>
    this.organizations.organizations().map((each) => place(each, this.locale)),
  );
  private readonly session = inject(SessionService);

  constructor() {
    // What the menu offers is the server's to say, whichever page opened first.
    void this.server.load();
    void this.organizations.load();
  }

  protected followed(): void {
    this.navigated.emit();
  }

  /** Works in `to` from now on. */
  protected switchTo(to: Place): void {
    const chosen = this.organizations.organizations().find((each) => each.id === to.id);
    if (chosen !== undefined && !to.current) {
      this.organizations.choose(chosen);
    }
  }

  protected invite(): void {
    this.inviting.set(true);
  }

  protected signOut(): void {
    void this.session.signOut();
  }
}
