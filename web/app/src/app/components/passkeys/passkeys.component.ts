import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  signal,
} from '@angular/core';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzConfirmDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';

import { LONGEST_NAME, PasskeysService } from '../../services/passkeys.service';

/**
 * A passkey as the card shows it: its name, when it was added and last used, and whether it
 * signed this browser in or was made from an emailed link.
 */
interface Shown {
  readonly id: string;
  readonly name: string;
  readonly detail: string;
}

/**
 * The person's passkeys, in Settings, where glave is reached from elsewhere or makes them on
 * this computer: each one, with when it was added and last used, adding one on this device,
 * and removing one.
 */
@Component({
  selector: 'app-passkeys',
  imports: [
    GlzButtonComponent,
    GlzCardComponent,
    GlzConfirmDialogComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
  ],
  templateUrl: './passkeys.component.html',
  styleUrl: './passkeys.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PasskeysComponent {
  private readonly keys = inject(PasskeysService);
  private readonly dates = new Intl.DateTimeFormat(inject(LOCALE_ID), { dateStyle: 'medium' });
  protected readonly copy = {
    heading: $localize`:@@passkeys.heading:Passkeys`,
    lead: $localize`:@@passkeys.lead:These sign you in to glave from your devices: your face, your fingerprint or a screen lock, never a password.`,
    none: $localize`:@@passkeys.none:No passkeys yet.`,
    add: $localize`:@@passkeys.add:Add a passkey on this device`,
    adding: $localize`:@@passkeys.adding:Waiting for your device…`,
    remove: $localize`:@@passkeys.remove:Remove`,
    removeHeading: $localize`:@@passkeys.removeHeading:Remove this passkey?`,
    removeText: $localize`:@@passkeys.removeText:It signs you in no more, and the browsers it signed in are signed out.`,
    email: $localize`:@@passkeys.email:Email, for getting back in`,
    yourName: $localize`:@@passkeys.yourName:Your name`,
    save: $localize`:@@passkeys.save:Save`,
    current: $localize`:@@passkeys.current:This device`,
    emailed: $localize`:@@passkeys.emailed:From an emailed link`,
  } as const;
  /** The address the person's glave account gets back in with, if it has one. */
  protected readonly email = this.keys.email;
  /** What the server tells of a passkey made by email lately, on another device. */
  protected readonly notice = this.keys.notice;
  /** Whether the person changes their name here, as the server says. */
  protected readonly nameable = this.keys.nameable;
  /** The name typed, until it is typed over the server's. */
  private readonly typed = signal<string | null>(null);
  protected readonly personName = computed(() => this.typed() ?? this.keys.personName() ?? '');
  protected readonly naming = signal(false);
  protected readonly longestName = LONGEST_NAME;
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly confirming = signal(false);
  private readonly removing = signal<string | null>(null);

  protected readonly shown = computed<readonly Shown[]>(() =>
    this.keys.passkeys().map((passkey) => {
      const added = this.dates.format(new Date(passkey.added));
      const last = passkey.used ?? null;
      const used = last === null ? null : this.dates.format(new Date(last));
      const detail =
        used === null
          ? $localize`:@@passkeys.added:Added ${added}:ADDED:`
          : $localize`:@@passkeys.addedUsed:Added ${added}:ADDED:, last used ${used}:USED:`;
      const marks = [
        ...(passkey.current === true ? [this.copy.current] : []),
        ...(passkey.emailed === true ? [this.copy.emailed] : []),
      ];
      // What else is said of it, after the dates: "This device", "From an emailed link".
      const marked = marks.reduce(
        (line, mark) => $localize`:@@passkeys.marked:${line}:LINE: · ${mark}:MARK:`,
        detail,
      );
      return { id: passkey.id, name: passkey.name, detail: marked };
    }),
  );
  protected readonly addLabel = computed(() => (this.busy() ? this.copy.adding : this.copy.add));

  constructor() {
    void this.keys.load().then((problem) => {
      this.problem.set(problem);
    });
  }

  protected typedName(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.typed.set(event.target.value);
    }
  }

  /** Changes the name the person goes by, saying why when the server will not. */
  protected async rename(event: Event): Promise<void> {
    event.preventDefault();
    this.naming.set(true);
    const problem = await this.keys.rename(this.personName());
    this.naming.set(false);
    this.problem.set(problem);
    if (problem === null) {
      this.typed.set(null);
    }
  }

  /** Adds a passkey on this device, saying why when it cannot. */
  protected async add(): Promise<void> {
    this.busy.set(true);
    this.problem.set(await this.keys.add());
    this.busy.set(false);
  }

  /** Asks before removing the passkey `id`. */
  protected ask(id: string): void {
    this.removing.set(id);
    this.confirming.set(true);
  }

  /** Removes the passkey asked about, when the person said yes. */
  protected async decided(yes: boolean): Promise<void> {
    const id = this.removing();
    this.removing.set(null);
    if (yes && id !== null) {
      this.problem.set(await this.keys.remove(id));
    }
  }
}
