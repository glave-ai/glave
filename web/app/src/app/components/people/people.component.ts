import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  signal,
} from '@angular/core';
import {
  GlzAvatarComponent,
  GlzButtonComponent,
  GlzCardComponent,
  GlzConfirmDialogComponent,
  GlzMenuSelectComponent,
} from '@glave/glaze';

import { type PersonRow, counted, inviteRow, isRole, personRow, untilIn } from '../../lib/people';
import { PeopleService } from '../../services/people.service';
import { InviteDialogComponent } from '../invite-dialog/invite-dialog.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

/**
 * Admin's People: the organization's people with their roles, each changed from a menu or taken
 * out (asked first), the invites waiting, each cancelled, and inviting someone. What the reader
 * may change of each person is the server's word.
 */
@Component({
  selector: 'app-people',
  imports: [
    InviteDialogComponent,
    PageHeaderComponent,
    GlzAvatarComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzConfirmDialogComponent,
    GlzMenuSelectComponent,
  ],
  templateUrl: './people.component.html',
  styleUrl: './people.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PeopleComponent {
  private readonly peopleService = inject(PeopleService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly copy = {
    heading: $localize`:@@people.heading:People`,
    list: $localize`:@@people.list:People`,
    invite: $localize`:@@people.invite:Invite people`,
    waiting: $localize`:@@people.waiting:Invites waiting`,
    cancelInvite: $localize`:@@people.cancelInvite:Cancel invite`,
    remove: $localize`:@@people.remove:Remove`,
    keep: $localize`:@@people.keep:Keep them`,
    removeText: $localize`:@@people.removeText:They're signed out right away, on every device. Their projects stay with the organization. Only a new invite brings them back.`,
  } as const;
  protected readonly problem = signal<string | null>(null);
  protected readonly inviting = signal(false);
  protected readonly confirming = signal(false);
  private readonly removing = signal<PersonRow | null>(null);

  private readonly days = new Intl.DateTimeFormat(this.locale, { dateStyle: 'medium' });
  protected readonly rows = computed(() =>
    this.peopleService
      .people()
      .map((person) => personRow(person, (at) => this.days.format(new Date(at)))),
  );
  protected readonly invites = computed(() => {
    const until = untilIn(this.locale, this.peopleService.readAt());
    const people = this.peopleService.people();
    return this.peopleService.invites().map((invite) => inviteRow(invite, people, until));
  });
  protected readonly hasInvites = computed(() => this.invites().length > 0);
  protected readonly count = computed(() =>
    counted(this.rows().length, this.invites().length, this.locale),
  );
  protected readonly removeHeading = computed(() => {
    const name = this.removing()?.name ?? '';
    return $localize`:@@people.removeHeading:Remove ${name}:NAME:?`;
  });

  constructor() {
    void this.peopleService.load().then((problem) => {
      this.problem.set(problem);
    });
  }

  /** Gives `person` the role chosen in their menu. */
  protected async setRole(person: string, role: string | null): Promise<void> {
    if (isRole(role)) {
      this.problem.set(await this.peopleService.setRole(person, role));
    }
  }

  /** Asks before taking `person` out. */
  protected ask(person: PersonRow): void {
    this.removing.set(person);
    this.confirming.set(true);
  }

  /** Takes out the person asked about, when the reader said yes. */
  protected async decided(yes: boolean): Promise<void> {
    const person = this.removing();
    if (yes && person !== null) {
      this.problem.set(await this.peopleService.remove(person.id));
    }
  }

  /** Cancels the invite `invite`. */
  protected async revoke(invite: string): Promise<void> {
    this.problem.set(await this.peopleService.revoke(invite));
  }

  protected openInvite(): void {
    this.inviting.set(true);
  }
}
