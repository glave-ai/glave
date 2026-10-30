import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  effect,
  inject,
  model,
  signal,
  untracked,
} from '@angular/core';
import type { OrgRole } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCopyButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { lasts } from '../../lib/people';
import { type Made, PeopleService } from '../../services/people.service';
import { ServerService } from '../../services/server.service';

/**
 * Inviting someone to the organization: their email, if the inviter has it, and what they can
 * do, Member or Admin, where the server shows the organization (alone in one it shows no one
 * yet, a Member); then the invite's link, with whether glave emailed it, to copy and send.
 */
@Component({
  selector: 'app-invite-dialog',
  imports: [
    GlzButtonComponent,
    GlzCopyButtonComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
  ],
  templateUrl: './invite-dialog.component.html',
  styleUrl: './invite-dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class InviteDialogComponent {
  /** Whether the dialog that makes an invite is open. */
  readonly open = model(false);

  private readonly people = inject(PeopleService);
  private readonly locale = inject(LOCALE_ID);
  private readonly server = inject(ServerService);
  private readonly page = inject(DOCUMENT).defaultView?.location.origin ?? '';
  protected readonly copy = {
    heading: $localize`:@@inviteDialog.heading:Invite someone`,
    email: $localize`:@@inviteDialog.email:Their email (optional)`,
    placeholder: $localize`:@@inviteDialog.placeholder:name@example.com`,
    role: $localize`:@@inviteDialog.role:What they can do`,
    member: $localize`:@@inviteDialog.member:Member`,
    memberHint: $localize`:@@inviteDialog.memberHint:Uses glave with what the team gives them.`,
    admin: $localize`:@@inviteDialog.admin:Admin`,
    adminHint: $localize`:@@inviteDialog.adminHint:Also adds and removes people and sets what everyone can use.`,
    cancel: $localize`:@@inviteDialog.cancel:Cancel`,
    make: $localize`:@@inviteDialog.make:Make invite link`,
    making: $localize`:@@inviteDialog.making:Making the link…`,
    ready: $localize`:@@inviteDialog.ready:Invite ready`,
    link: $localize`:@@inviteDialog.link:Invite link`,
    copyLink: $localize`:@@inviteDialog.copyLink:Copy link`,
    done: $localize`:@@inviteDialog.done:Done`,
  } as const;
  protected readonly email = signal('');
  protected readonly role = signal<OrgRole>('member');
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly made = signal<Made | null>(null);
  protected readonly showing = signal(false);
  /**
   * Whether the inviter chooses what they can do: where the server shows them the organization
   * they are an admin of, as it said when the dialog opened. Alone, with none shown yet, the
   * first is a Member.
   */
  protected readonly chooses = signal(false);
  /** Whether glave sends the link to an address: where it doesn't, none is asked for. */
  protected readonly emails = this.server.email;
  protected readonly makeLabel = computed(() => (this.busy() ? this.copy.making : this.copy.make));
  protected readonly link = computed(() => this.made()?.link ?? '');
  /** What the ready dialog says: whether glave emailed the link, and how long it works. */
  protected readonly told = computed(() => {
    const made = this.made();
    if (made === null) {
      return '';
    }
    const works = lasts(made.invite, this.locale);
    const to = made.invite.email ?? null;
    return to === null
      ? $localize`:@@inviteDialog.share:Send the link yourself. It works once, for ${works}:LASTS:.`
      : $localize`:@@inviteDialog.emailed:We emailed it to ${to}:EMAIL:. You can also send the link yourself. It works once, for ${works}:LASTS:.`;
  });

  constructor() {
    // Each time it opens, it starts empty, a Member.
    effect(() => {
      if (this.open()) {
        this.email.set('');
        this.role.set('member');
        this.problem.set(null);
        // As it is now: the invite this dialog makes changes it, for the next.
        this.chooses.set(untracked(this.server.admin));
      }
    });
  }

  protected typed(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.email.set(event.target.value);
    }
  }

  protected chose(value: string | null): void {
    this.role.set(value === 'admin' ? 'admin' : 'member');
  }

  /** Makes the invite, then shows its link; or says why it was not made. */
  protected async make(event?: Event): Promise<void> {
    event?.preventDefault();
    if (this.busy()) {
      return;
    }
    this.busy.set(true);
    const email = this.email().trim();
    const origin = this.server.origin() ?? this.page;
    const made = await this.people.invite(this.role(), email === '' ? null : email, origin);
    this.busy.set(false);
    if (typeof made === 'string') {
      this.problem.set(made);
      return;
    }
    this.made.set(made);
    this.open.set(false);
    this.showing.set(true);
  }

  protected cancel(): void {
    this.open.set(false);
  }

  protected done(): void {
    this.showing.set(false);
    this.made.set(null);
  }
}
