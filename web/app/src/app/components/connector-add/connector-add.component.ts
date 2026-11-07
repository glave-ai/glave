import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  input,
  model,
  output,
  signal,
  untracked,
} from '@angular/core';
import type { ConnectorInfo, ProjectId } from '@glave/client';
import {
  GlzButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  type GlzOption,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { ACCOUNT_NAME_LIMIT } from '../../lib/connections';

/** What the person chose: an account of the service and where it is used. */
export interface AccountChosen {
  /** Whether it is one already connected, moved here, or another to sign in to. */
  readonly existing: boolean;
  /**
   * Its name: none for the one never named, or, another, for one the server names from its
   * sign-in.
   */
  readonly account: string | null;
  /** The one place it is used, or none for everywhere. */
  readonly only: ProjectId | null;
}

/** The choice of a new account to sign in to, beside each connected one's. */
const ANOTHER = '\u0000another';
/** The choice of every place. */
const EVERYWHERE = 'everywhere';

/** A connected account as a choice of the dialog. */
interface Had {
  readonly value: string;
  readonly name: string | null;
  readonly label: string;
  /** Who it is signed in as, in the server's words, else that it is connected already. */
  readonly hint: string;
}

/**
 * Connecting another account of a service the person has: which account (one connected
 * already, to use here, each with who it is signed in as, or another to sign in to), its name,
 * which the server takes from its sign-in when it is left empty, and where it is used. From
 * inside a project it starts on only that project, which nothing else can see; elsewhere, on
 * everywhere. The server says whether the name and the place may be had.
 */
@Component({
  selector: 'app-connector-add',
  imports: [
    GlzButtonComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
  ],
  templateUrl: './connector-add.component.html',
  styleUrl: './connector-add.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConnectorAddComponent {
  /** Whether the dialog is open. */
  readonly open = model(false);
  /** The service, with the accounts of it connected. */
  readonly connector = input<ConnectorInfo | null>(null);
  /** The project the page is for, where it starts, and its name; none for Settings. */
  readonly here = input<ProjectId | null>(null);
  readonly hereName = input('');
  /** Every place the person has, for Settings: everywhere first, then each project. */
  readonly places = input<readonly GlzOption[]>([]);
  /** Emitted with what was chosen, as the button is pressed, so a sign-in's tab opens then. */
  readonly chosen = output<AccountChosen>();

  protected readonly copy = {
    account: $localize`:@@connectorAdd.account:Account`,
    already: $localize`:@@connectorAdd.already:Already connected`,
    nameIt: $localize`:@@connectorAdd.nameIt:Name it (optional)`,
    nameHint: $localize`:@@connectorAdd.nameHint:Such as the company it's for.`,
    useIt: $localize`:@@connectorAdd.useIt:Use it`,
    nothingElse: $localize`:@@connectorAdd.nothingElse:Nothing else can see it`,
    everywhere: $localize`:@@connectorAdd.everywhere:Everywhere, including chats`,
    cancel: $localize`:@@connectorAdd.cancel:Cancel`,
    connect: $localize`:@@connectorAdd.connect:Connect`,
    use: $localize`:@@connectorAdd.use:Use it`,
  } as const;
  protected readonly service = computed(() => this.connector()?.name ?? '');
  protected readonly heading = computed(() => {
    const service = this.service();
    return $localize`:@@connectorAdd.heading:Connect ${service}:SERVICE:`;
  });
  protected readonly another = ANOTHER;
  /** The longest name the server takes. */
  protected readonly longest = ACCOUNT_NAME_LIMIT;
  protected readonly anotherLabel = computed(() => {
    const service = this.service();
    return $localize`:@@connectorAdd.another:Sign in to another ${service}:SERVICE: account`;
  });
  /** What an empty name becomes, as the field says before anything is typed. */
  protected readonly namePlaceholder = computed(() => {
    const service = this.service();
    return $localize`:@@connectorAdd.namePlaceholder:From your ${service}:SERVICE: sign-in, such as Acme Corp`;
  });
  /** Each account connected, by its name or the service's. */
  protected readonly had = computed((): readonly Had[] =>
    (this.connector()?.accounts ?? []).map((it, index) => ({
      value: String(index),
      name: it.name ?? null,
      label: it.name ?? this.service(),
      hint: it.signed_in ?? this.copy.already,
    })),
  );
  /** Where it may be used: from a project, there or everywhere; else every place. */
  protected readonly where = computed((): readonly (GlzOption & { readonly hint: string })[] => {
    const here = this.here();
    if (here === null) {
      return this.places().map((it) => ({ ...it, hint: '' }));
    }
    const name = this.hereName();
    return [
      {
        value: here,
        label: $localize`:@@connectorAdd.onlyIn:Only in ${name}:PLACE:`,
        hint: this.copy.nothingElse,
      },
      { value: EVERYWHERE, label: this.copy.everywhere, hint: '' },
    ];
  });

  protected readonly account = signal<string>(ANOTHER);
  protected readonly name = signal('');
  protected readonly place = signal<string>(EVERYWHERE);
  protected readonly problem = signal<string | null>(null);
  protected readonly naming = computed(() => this.account() === ANOTHER);
  /** The button's word: signing in to the service, connecting one that asks no one, or using one. */
  protected readonly action = computed(() => {
    if (!this.naming()) {
      return this.copy.use;
    }
    const service = this.service();
    return this.connector()?.sign_in === 'dcr'
      ? $localize`:@@connectorAdd.signIn:Sign in to ${service}:SERVICE:`
      : this.copy.connect;
  });

  constructor() {
    // Each time it opens, it starts on another account, where the page is.
    effect(() => {
      if (this.open()) {
        untracked(() => {
          this.account.set(ANOTHER);
          this.name.set('');
          this.place.set(this.free());
          this.problem.set(null);
        });
      }
    });
  }

  /**
   * Where a new account starts: where the page is, or, from Settings, the first place no
   * account of the service is used in, since no two are used in one.
   */
  private free(): string {
    const here = this.here();
    if (here !== null) {
      return here;
    }
    const used = (this.connector()?.accounts ?? []).map((it) => it.only ?? EVERYWHERE);
    const open = this.where().find((it) => !used.includes(it.value));
    return open?.value ?? EVERYWHERE;
  }

  protected choseAccount(value: string | null): void {
    this.account.set(value ?? ANOTHER);
    this.problem.set(null);
  }

  protected chosePlace(value: string | null): void {
    this.place.set(value ?? EVERYWHERE);
  }

  protected typed(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.name.set(event.target.value);
      this.problem.set(null);
    }
  }

  /** Says what was chosen, and closes; a new account left unnamed is named by the server. */
  protected done(event?: Event): void {
    event?.preventDefault();
    const place = this.place();
    const only: ProjectId | null = place === EVERYWHERE ? null : place;
    if (this.naming()) {
      const name = this.name().trim();
      this.chosen.emit({ existing: false, account: name === '' ? null : name, only });
    } else {
      const had = this.had().find((it) => it.value === this.account());
      this.chosen.emit({ existing: true, account: had?.name ?? null, only });
    }
    this.open.set(false);
  }

  protected cancel(): void {
    this.open.set(false);
  }
}
