import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  output,
} from '@angular/core';
import type { ConnectorAccount, ConnectorInfo, ProjectId } from '@glave/client';
import {
  type GlzButtonVariant,
  GlzButtonComponent,
  GlzIconComponent,
  GlzMenuButtonComponent,
  type GlzMenuOption,
  type GlzOption,
} from '@glave/glaze';
import { ExternalLink, LoaderCircle } from 'lucide';

import { signInWaiting } from '../../lib/connections';
import { plural } from '../../lib/plural';
import {
  type AccountAction,
  ConnectorAccountComponent,
} from '../connector-account/connector-account.component';
import { ServiceRowComponent } from '../service-row/service-row.component';

/** A sign-in waiting for the person in the browser. */
export interface Waiting {
  /** The vendor's sign-in page, once the server gave it. */
  readonly link: string | null;
  /** Whether a new tab is open at it: none where the browser opened none, or it was closed. */
  readonly opened: boolean;
}

/**
 * What the person asked of an account of the connector, by its name (none: the one never
 * named): connect it, used only in `only` (by default where the page is), stop its sign-in, use
 * it in `only` from now on (none: everywhere), add another, rename it (the name it starts from
 * as `called`) or disconnect it (as `called`).
 */
export type ConnectorAction =
  | { readonly kind: 'connect'; readonly account: string | null; readonly only?: ProjectId | null }
  | { readonly kind: 'cancel' }
  | { readonly kind: 'place'; readonly account: string | null; readonly only: ProjectId | null }
  | { readonly kind: 'add' }
  | {
      readonly kind: 'rename';
      readonly account: string | null;
      readonly called: string;
      /** The dialog's heading and note, where the row has its own. */
      readonly heading: string | null;
      readonly note: string | null;
    }
  | { readonly kind: 'disconnect'; readonly account: string | null; readonly called: string };

/** The choice of where a connector is used that stands for every place. */
export const EVERYWHERE = 'everywhere';

/** The values of a lone account's menu beside each place's own. */
const NAME = '\u0000name';
const ADD = '\u0000add';
const DISCONNECT = '\u0000disconnect';

/** A connector as its row shows it. */
interface Row {
  readonly name: string;
  /** Whether it is connected, ready to use. */
  readonly connected: boolean;
  /** Whether it is connected but to be signed in to again: its Disconnect shows. */
  readonly again: boolean;
  /** Whether it waits for the person to sign in to it in the browser. */
  readonly waiting: boolean;
  /** Whether it is being connected or disconnected. */
  readonly busy: boolean;
  /** The line under its name. */
  readonly detail: string;
  /** The sign-in's page, to open while it waits, once the server gave it, and the link's word. */
  readonly link: string | null;
  readonly linkText: string;
  /** Its button's word, its name for screen readers, and how much it stands out. */
  readonly action: string;
  readonly actionLabel: string;
  readonly variant: GlzButtonVariant;
  /** Disconnect's name for screen readers, for a row to sign in to again. */
  readonly disconnectLabel: string;
  /** Connected, its one account's place: `everywhere`, or the project its threads alone use. */
  readonly place: string;
  /**
   * Connected with one account, its menu (each other place, naming it, adding another, and
   * disconnecting) and the menu's name; else none. Where it is used shows at the row's end
   * while the line under its name says who it is signed in as.
   */
  readonly menu: readonly GlzOption[];
  readonly menued: boolean;
  readonly menuLabel: string;
  readonly where: string | null;
  /** The account the row moves, signs in to again and disconnects, with one: by its name, none unnamed. */
  readonly account: string | null;
  /** Where that account is used alone, or none for everywhere. */
  readonly only: ProjectId | null;
  /** What its moving and disconnecting call it: its name, with the account's when named. */
  readonly called: string;
  /** With two accounts or more, each beneath it, with its own menu; else none. */
  readonly accounts: readonly Beneath[];
  readonly listed: boolean;
  /** Whether its own button shows: for one with two accounts or more, each account's menu acts. */
  readonly acts: boolean;
  /** Connected, its button to add another account, and that button's name for screen readers. */
  readonly adds: boolean;
  readonly addLabel: string;
}

/** An account beneath its service, and its place as the menus' value. */
interface Beneath {
  readonly account: ConnectorAccount;
  readonly place: string;
  /** What tracks it among its service's accounts: its name, unique among them. */
  readonly key: string;
}

/**
 * One connector on the Connections page: its name and what it is doing (why the last thing
 * done with it failed, its sign-in waiting, or where it is used), with two accounts or more each
 * beneath it with its own menu, and at its end where it is used, Add account, and its button,
 * which connects it, stops its sign-in, signs in to it again or disconnects it. The page does
 * what the person asks of it.
 */
@Component({
  selector: 'app-connector-row',
  imports: [
    ServiceRowComponent,
    GlzButtonComponent,
    GlzIconComponent,
    GlzMenuButtonComponent,
    ConnectorAccountComponent,
  ],
  templateUrl: './connector-row.component.html',
  styleUrl: './connector-row.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConnectorRowComponent {
  readonly connector = input.required<ConnectorInfo>();
  /** Its sign-in waiting for the person, if one is. */
  readonly waiting = input<Waiting | null>(null);
  /** Whether a call for it runs. */
  readonly working = input(false);
  /** Why the last thing done with it failed, in the server's words. */
  readonly problem = input<string | null>(null);
  /** Whether the person has somewhere to choose: with only their Chats, no places. */
  readonly choosing = input(false);
  /** Where a connector can be used: everywhere, or only in one of the person's places. */
  readonly places = input<readonly GlzMenuOption[]>([]);
  /** What the person asked of it. */
  readonly acted = output<ConnectorAction>();

  protected readonly copy = {
    connected: $localize`:@@connectorRow.connected:Connected`,
    signInAgain: $localize`:@@connectorRow.signInAgain:Sign in again to keep using it`,
    connect: $localize`:@@connectorRow.connect:Connect`,
    connecting: $localize`:@@connectorRow.connecting:Connecting…`,
    again: $localize`:@@connectorRow.again:Sign in again`,
    cancel: $localize`:@@connectorRow.cancel:Cancel`,
    disconnect: $localize`:@@connectorRow.disconnect:Disconnect`,
    disconnecting: $localize`:@@connectorRow.disconnecting:Disconnecting…`,
    openAgain: $localize`:@@connectorRow.openAgain:Open it again`,
    openSignIn: $localize`:@@connectorRow.openSignIn:Open the sign-in`,
    newTab: $localize`:@@connectorRow.newTab:in a new tab`,
    add: $localize`:@@connectorRow.add:Add account`,
    name: $localize`:@@connectorRow.name:Name this account`,
  } as const;
  protected readonly icons = { waiting: LoaderCircle, open: ExternalLink } as const;
  private readonly locale = inject(LOCALE_ID);
  protected readonly row = computed(() => this.drawn());
  /** For a project, why the account used everywhere else is not used here, in the server's words. */
  protected readonly note = computed(() => this.connector().replaced ?? null);

  /** Does what was picked from its one account's menu. */
  protected chose(value: string): void {
    const { account, busy } = this.row();
    if (busy) {
      return;
    }
    if (value === NAME) {
      const { status } = this.connector();
      const one = this.connector().accounts?.[0] ?? { status, used: '' };
      this.acted.emit(this.renaming(one, true));
    } else if (value === ADD) {
      this.add();
    } else if (value === DISCONNECT) {
      this.ask();
    } else {
      this.acted.emit({ kind: 'place', account, only: value === EVERYWHERE ? null : value });
    }
  }

  /** What its button does: stops waiting, disconnects, or connects (again, where it is). */
  protected act(): void {
    const row = this.row();
    if (row.busy) {
      return;
    }
    if (row.waiting) {
      this.acted.emit({ kind: 'cancel' });
    } else if (row.connected) {
      this.ask();
    } else if (row.again) {
      this.acted.emit({ kind: 'connect', account: row.account, only: row.only });
    } else {
      this.acted.emit({ kind: 'connect', account: null });
    }
  }

  /** Asks to disconnect its one account. */
  protected ask(): void {
    const { account, called } = this.row();
    this.acted.emit({ kind: 'disconnect', account, called });
  }

  protected add(): void {
    this.acted.emit({ kind: 'add' });
  }

  /** Does what was picked from the menu of `account`, beneath it. */
  protected picked(account: ConnectorAccount, action: AccountAction): void {
    const service = this.connector().name;
    const name = account.name ?? null;
    if (action.kind === 'place') {
      const only = action.place === EVERYWHERE ? null : action.place;
      this.acted.emit({ kind: 'place', account: name, only });
    } else if (action.kind === 'again') {
      this.acted.emit({ kind: 'connect', account: name, only: account.only ?? null });
    } else if (action.kind === 'rename') {
      this.acted.emit(this.renaming(account, false));
    } else {
      const called = name === null ? service : this.called(service, name);
      this.acted.emit({ kind: 'disconnect', account: name, called });
    }
  }

  /**
   * Renaming `account`, the service's `one` account or one beneath it: it starts from its name,
   * else who it is signed in as, which the dialog's note says, else the service's. Its one
   * account is named as the mockup's "Name this account", shown once there is a second.
   */
  private renaming(account: ConnectorAccount, one: boolean): ConnectorAction {
    const service = this.connector().name;
    const name = account.name ?? null;
    const filled = name === null && (account.who ?? null) !== null;
    let note: string | null = null;
    if (one && filled) {
      note = $localize`:@@connectorRow.filledOne:Filled in from your ${service}:SERVICE: sign-in. Shown when you add a second ${service}:SERVICE: account.`;
    } else if (one) {
      note = $localize`:@@connectorRow.shownOne:Shown when you add a second ${service}:SERVICE: account.`;
    } else if (filled) {
      note = $localize`:@@connectorRow.filled:Filled in from your ${service}:SERVICE: sign-in.`;
    }
    const heading = one
      ? $localize`:@@connectorRow.nameHeading:Name this ${service}:SERVICE: account`
      : null;
    const called = name ?? account.who ?? service;
    return { kind: 'rename', account: name, called, heading, note };
  }

  /** Its one account's menu: each other place, naming it, adding another, and disconnecting. */
  private menuOf(place: string): readonly GlzOption[] {
    const service = this.connector().name;
    const places = this.choosing() ? this.places().filter((it) => it.value !== place) : [];
    return [
      ...places.map((it) => ({ value: it.value, label: it.label })),
      { value: NAME, label: this.copy.name },
      {
        value: ADD,
        label: $localize`:@@connectorRow.addAnother:Add another ${service}:SERVICE: account`,
      },
      { value: DISCONNECT, label: this.copy.disconnect },
    ];
  }

  /** What the row calls the account `account` of `service` where it acts on it: `Stripe (Acme)`. */
  private called(service: string, account: string): string {
    return $localize`:@@connectorRow.called:${service}:SERVICE: (${account}:ACCOUNT:)`;
  }

  /** The row's title for the one account of `service` it shows, named: `Stripe: Acme`. */
  private titled(service: string, account: string): string {
    return $localize`:@@connectorRow.titled:${service}:SERVICE:: ${account}:ACCOUNT:`;
  }

  /** The connector as its row: what it is, what it is doing, and what its button does. */
  private drawn(): Row {
    const connector = this.connector();
    const { name, status } = connector;
    const connected = status === 'connected';
    const again = status === 'sign_in_again';
    const waited = this.waiting();
    const busy = this.working() && waited === null;
    const all = connector.accounts ?? [];
    const [first] = all;
    const several = all.length > 1;
    const account = first?.name ?? null;
    const called = account === null ? name : this.called(name, account);
    const base = {
      // Named only beside another account: where the project's own takes the place of another.
      name: account === null || several || this.note() === null ? name : this.titled(name, account),
      connected,
      again,
      busy,
      waiting: false,
      link: null,
      linkText: '',
      disconnectLabel: $localize`:@@connectorRow.disconnectLabel:Disconnect ${called}:NAME:`,
      place: first?.only ?? EVERYWHERE,
      menu: [],
      menued: false,
      menuLabel: $localize`:@@connectorRow.menu:Options for ${called}:NAME:`,
      where: null,
      account,
      only: first?.only ?? null,
      called,
      accounts: several
        ? all.map((it) => ({ account: it, place: it.only ?? EVERYWHERE, key: it.name ?? '' }))
        : [],
      listed: several,
      acts: !several,
      // A lone account's menu adds another; one to sign in to again is signed in to first.
      adds: several,
      addLabel: $localize`:@@connectorRow.addLabel:Add account to ${name}:NAME:`,
    };
    // A sign-in waiting shows first: a connected one's second account's, or one signed in again.
    if (waited !== null) {
      return {
        ...base,
        waiting: true,
        acts: true,
        // Its Cancel stops the sign-in; Disconnect would not know which account to take.
        again: again && !several,
        detail: signInWaiting(name, waited.opened),
        link: waited.link,
        linkText: waited.opened ? this.copy.openAgain : this.copy.openSignIn,
        action: this.copy.cancel,
        actionLabel: $localize`:@@connectorRow.cancelLabel:Cancel connecting ${name}:NAME:`,
        variant: 'secondary',
      };
    }
    if (several) {
      const count = all.length;
      return {
        ...base,
        again: false,
        detail: plural(
          $localize`:@@connectorRow.accounts:{${count}:COUNT:, plural, one {# account} other {# accounts}}`,
          this.locale,
        ),
        action: '',
        actionLabel: '',
        variant: 'ghost',
      };
    }
    if (connected) {
      const signedIn = first?.signed_in ?? null;
      return {
        ...base,
        acts: false,
        menu: this.menuOf(base.place),
        menued: true,
        detail: signedIn ?? first?.used ?? this.copy.connected,
        where: signedIn === null ? null : (first?.used ?? null),
        action: '',
        actionLabel: '',
        variant: 'ghost',
      };
    }
    return {
      ...base,
      detail: again ? this.copy.signInAgain : connector.description,
      action: busy ? this.copy.connecting : again ? this.copy.again : this.copy.connect,
      actionLabel: $localize`:@@connectorRow.connectLabel:Connect ${name}:NAME:`,
      variant: 'secondary',
    };
  }
}
