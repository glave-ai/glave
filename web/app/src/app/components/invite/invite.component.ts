import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import type { OrgRole, ShownInvite } from '@glave/client';
import { GlzButtonComponent, GlzEmptyStateComponent, GlzTopBarComponent } from '@glave/glaze';
import MailOpen from 'lucide/dist/esm/icons/mail-open.mjs';

import { InviteService } from '../../services/invite.service';

/** What the page asks of the person, from what the invite opens and whether they signed in. */
type Asked = 'reading' | 'gone' | 'failed' | 'passkey' | 'company' | 'join' | 'member';

/** The role an invite gives, in the sentence that says how the person joins. */
function joinsAs(role: OrgRole): string {
  switch (role) {
    case 'owner':
      return $localize`:@@invite.asOwner:You'll join as an owner.`;
    case 'admin':
      return $localize`:@@invite.asAdmin:You'll join as an admin.`;
    case 'member':
      return $localize`:@@invite.asMember:You'll join as a member.`;
  }
}

/** Who invited the person, as the page's heading says it. */
function invitedBy(shown: ShownInvite): string {
  const by = shown.by ?? null;
  return by === null
    ? $localize`:@@invite.invited:You're invited to glave`
    : $localize`:@@invite.invitedBy:${by}:NAME: invited you to glave`;
}

/**
 * The page an invite's link opens (`#invite=…`), signed in or not, drawn from what the server
 * says of it and of the reader. New to glave, the person makes a passkey and joins; where the
 * team signs people in with its own login, they continue there; signed in, they join the team
 * as one more of theirs, or not now, or are told they are in it already; and an invite that
 * no longer opens anything says so, and to ask for a new one.
 */
@Component({
  selector: 'app-invite',
  imports: [RouterLink, GlzButtonComponent, GlzEmptyStateComponent, GlzTopBarComponent],
  templateUrl: './invite.component.html',
  styleUrl: './invite.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class InviteComponent {
  private readonly invite = inject(InviteService);
  private readonly router = inject(Router);
  protected readonly copy = {
    name: 'glave',
    passkeyText: $localize`:@@invite.passkeyText:Make a passkey to join. It signs you in with your fingerprint, face or device code. No password to remember.`,
    passkeyAction: $localize`:@@invite.passkeyAction:Make a passkey and join`,
    companyText: $localize`:@@invite.companyText:Your team signs you in with its own login. Use your work account.`,
    companyAction: $localize`:@@invite.companyAction:Continue with your team's sign-in`,
    joinHeading: $localize`:@@invite.joinHeading:Join this team?`,
    joinText: $localize`:@@invite.joinText:It gets its own chats and projects, apart from yours. Switch between them from your menu.`,
    memberHeading: $localize`:@@invite.memberHeading:You're in this team already`,
    memberText: $localize`:@@invite.memberText:This invite is for someone who isn't. Nothing to do here.`,
    have: $localize`:@@invite.have:Already have an account?`,
    signIn: $localize`:@@invite.signIn:Sign in, then open this link again`,
    join: $localize`:@@invite.join:Join`,
    notNow: $localize`:@@invite.notNow:Not now`,
    goneHeading: $localize`:@@invite.goneHeading:This invite doesn't work anymore`,
    goneText: $localize`:@@invite.goneText:It was already used, cancelled, or more than 7 days old. Ask whoever sent it for a new one.`,
    open: $localize`:@@invite.open:Go to glave`,
    waiting: $localize`:@@invite.waiting:Waiting for your device…`,
    joining: $localize`:@@invite.joining:Joining…`,
  } as const;
  protected readonly icon = MailOpen;
  protected readonly signInLink = '/sign-in';
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected readonly asked = computed<Asked>(() => {
    const opened = this.invite.opened();
    if (opened.kind !== 'shown') {
      return opened.kind;
    }
    switch (opened.shown.joining) {
      case 'accept':
        return 'join';
      default:
        return opened.shown.joining;
    }
  });
  private readonly shown = computed(() => {
    const opened = this.invite.opened();
    return opened.kind === 'shown' ? opened.shown : null;
  });
  protected readonly heading = computed(() => {
    const shown = this.shown();
    switch (this.asked()) {
      case 'gone':
        return this.copy.goneHeading;
      case 'join':
        return this.copy.joinHeading;
      case 'member':
        return this.copy.memberHeading;
      default:
        return shown === null ? '' : invitedBy(shown);
    }
  });
  /** Who invited the person, said once more above what joining does, when signed in. */
  protected readonly byLine = computed(() => {
    const by = this.shown()?.by ?? null;
    return by === null ? null : $localize`:@@invite.byLine:${by}:NAME: invited you.`;
  });
  protected readonly role = computed(() => {
    const shown = this.shown();
    return shown === null ? '' : joinsAs(shown.role);
  });
  protected readonly failed = computed(() => {
    const opened = this.invite.opened();
    return opened.kind === 'failed' ? opened.problem : null;
  });
  protected readonly passkeyLabel = computed(() =>
    this.busy() ? this.copy.waiting : this.copy.passkeyAction,
  );
  protected readonly joinLabel = computed(() => (this.busy() ? this.copy.joining : this.copy.join));

  constructor() {
    void this.invite.load();
  }

  /** Makes a passkey and joins, then opens glave. */
  protected async makePasskey(): Promise<void> {
    await this.doing(() => this.invite.joinWithPasskey());
  }

  /** Joins as the person signed in, then opens glave. */
  protected async join(): Promise<void> {
    await this.doing(() => this.invite.accept());
  }

  /** Goes on to the team's own sign-in, the invite kept for when they come back signed in. */
  protected async company(): Promise<void> {
    await this.router.navigateByUrl('/sign-in');
  }

  /** Puts the invite aside and opens glave. */
  protected async later(): Promise<void> {
    this.invite.later();
    await this.router.navigateByUrl('/');
  }

  private async doing(act: () => Promise<string | null>): Promise<void> {
    if (this.busy()) {
      return;
    }
    this.busy.set(true);
    const problem = await act();
    this.busy.set(false);
    this.problem.set(problem);
    if (problem === null) {
      await this.router.navigateByUrl('/');
    }
  }
}
