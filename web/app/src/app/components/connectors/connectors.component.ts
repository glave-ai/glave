import { LiveAnnouncer } from '@angular/cdk/a11y';
import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  Injector,
  LOCALE_ID,
  afterNextRender,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
} from '@angular/core';
import type { ConnectorAccount, ConnectorInfo, ProjectId } from '@glave/client';
import {
  type GlzButtonVariant,
  GlzButtonComponent,
  GlzConfirmDialogComponent,
  GlzIconComponent,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { ExternalLink, LoaderCircle, Search } from 'lucide';

import { connectionProblem, openedTab, sentTo } from '../../lib/connections';
import { plural } from '../../lib/plural';
import { ConnectorsService } from '../../services/connectors.service';
import { ProjectsService } from '../../services/projects.service';
import {
  type AccountAction,
  ConnectorAccountComponent,
} from '../connector-account/connector-account.component';
import {
  type AccountChosen,
  ConnectorAddComponent,
} from '../connector-add/connector-add.component';
import { ServiceRowComponent } from '../service-row/service-row.component';

/** A sign-in waiting for the person in the browser. */
interface Waiting {
  /** The vendor's sign-in page, once the server gave it. */
  readonly link: string | null;
  /** Whether a new tab is open at it: none where the browser opened none, or it was closed. */
  readonly opened: boolean;
}

/** A connector as a row of the page shows it. */
interface Row {
  readonly id: string;
  readonly name: string;
  /** Whether it is connected, ready to use. */
  readonly connected: boolean;
  /** Whether it is connected but to be signed in to again. */
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
  /** Why the last thing done with it failed, in the server's words. */
  readonly problem: string | null;
  /** Its button's word, its name for screen readers, and how much it stands out. */
  readonly action: string;
  readonly actionLabel: string;
  readonly variant: GlzButtonVariant;
  /** Disconnect's name for screen readers, for a row to sign in to again. */
  readonly disconnectLabel: string;
  /** Connected, where it is used: `everywhere`, or the project its threads alone use. */
  readonly place: string | null;
  /** The account the row moves, signs in to again and disconnects, with one: by its name, none unnamed. */
  readonly account: string | null;
  /** Where that account is used alone, or none for everywhere. */
  readonly only: ProjectId | null;
  /** What its moving and disconnecting call it: its name, with the account's when named. */
  readonly called: string;
  /** The choice of where it is used, as screen readers name it. */
  readonly placeLabel: string;
  /** With two accounts or more, each beneath it, with its own menu; else none. */
  readonly accounts: readonly Beneath[];
  /** Whether it has accounts beneath it, and its tile's name, the service's own. */
  readonly listed: boolean;
  readonly tile: string;
  /** Whether its own button shows: for one with two accounts or more, each account's menu acts. */
  readonly acts: boolean;
  /** Connected, its button to add another account, and that button's name for screen readers. */
  readonly adds: boolean;
  readonly addLabel: string;
  /** For a project, why the account used everywhere else is not used here, in the server's words. */
  readonly note: string | null;
  readonly noted: boolean;
}

/** An account beneath its service, and its place as the menus' value. */
interface Beneath {
  readonly account: ConnectorAccount;
  readonly place: string;
}

/** What Disconnect asks about: an account of a connector, as the page calls it. */
interface Disconnecting {
  readonly id: string;
  readonly name: string;
  readonly account: string | null;
  readonly called: string;
}

/** The choice of where a connector is used that stands for every place. */
const EVERYWHERE = 'everywhere';

/** A card of the page: its heading, and its connectors. */
interface Section {
  readonly id: string;
  readonly testId: string;
  readonly heading: string;
  readonly rows: readonly Row[];
}

/** `map` with `key` set to `value`, or without it when `value` is null. */
function setTo<T>(
  map: ReadonlyMap<string, T>,
  key: string,
  value: T | null,
): ReadonlyMap<string, T> {
  const next = new Map(map);
  if (value === null) {
    next.delete(key);
  } else {
    next.set(key, value);
  }
  return next;
}

/**
 * glave's connectors, on the Connections page: a search over them, those connected first, each
 * with where it is used, everywhere or only in one project or Chats, which the person changes
 * there, and Disconnect, which asks first; for a `project`, only those its threads use, and one
 * connected here is used only there; then the rest by category, each with Connect, which
 * connects one that asks no one to sign in at once, and otherwise opens its sign-in in a new tab
 * and waits until the server says it is done. Names, descriptions and categories are the
 * server's; the page writes only its own labels for a connector's status.
 */
@Component({
  selector: 'app-connectors',
  imports: [
    ServiceRowComponent,
    GlzButtonComponent,
    GlzConfirmDialogComponent,
    GlzIconComponent,
    GlzMenuSelectComponent,
    ConnectorAccountComponent,
    ConnectorAddComponent,
  ],
  templateUrl: './connectors.component.html',
  styleUrl: './connectors.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConnectorsComponent {
  protected readonly copy = {
    heading: $localize`:@@connectors.heading:Apps`,
    search: $localize`:@@connectors.search:Search apps`,
    connected: $localize`:@@connectors.connected:Connected`,
    signInAgain: $localize`:@@connectors.signInAgain:Sign in again to keep using it`,
    connect: $localize`:@@connectors.connect:Connect`,
    connecting: $localize`:@@connectors.connecting:Connecting…`,
    again: $localize`:@@connectors.again:Sign in again`,
    cancel: $localize`:@@connectors.cancel:Cancel`,
    disconnect: $localize`:@@connectors.disconnect:Disconnect`,
    disconnecting: $localize`:@@connectors.disconnecting:Disconnecting…`,
    disconnected: $localize`:@@connectors.disconnected:Disconnected`,
    openAgain: $localize`:@@connectors.openAgain:Open it again`,
    openSignIn: $localize`:@@connectors.openSignIn:Open the sign-in`,
    newTab: $localize`:@@connectors.newTab:in a new tab`,
    disconnectText: $localize`:@@connectors.disconnectText:glave stops using it for you and forgets any sign-in to it. You can connect it again at any time.`,
    noMatch: $localize`:@@connectors.noMatch:No app matches that search.`,
    everywhere: $localize`:@@connectors.everywhere:Everywhere, including chats`,
    change: $localize`:@@connectors.change:Change`,
    thisProject: $localize`:@@connectors.thisProject:this project`,
    elsewhere: $localize`:@@connectors.elsewhere:Connected elsewhere`,
    add: $localize`:@@connectors.add:Add account`,
  } as const;
  /** The project the page is for, whose threads alone use what is connected from it. */
  readonly project = input<ProjectId | null>(null);
  protected readonly icons = { waiting: LoaderCircle, search: Search, open: ExternalLink } as const;
  private readonly service = inject(ConnectorsService);
  private readonly projects = inject(ProjectsService);
  private readonly document = inject(DOCUMENT);
  private readonly announcer = inject(LiveAnnouncer);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly injector = inject(Injector);
  private readonly locale = inject(LOCALE_ID);
  /** What the person searches the connectors for. */
  protected readonly query = signal('');
  /** What stops each sign-in waiting, by the connector's id. */
  private readonly stops = new Map<string, AbortController>();
  /** The sign-ins waiting, by the connector's id. */
  private readonly waiting = signal<ReadonlyMap<string, Waiting>>(new Map());
  /** The connectors being connected or disconnected. */
  private readonly working = signal<ReadonlyMap<string, true>>(new Map());
  /** Why the last thing done with a connector failed, by its id. */
  private readonly problems = signal<ReadonlyMap<string, string>>(new Map());
  /** The connector Disconnect asks about. */
  private readonly disconnecting = signal<Disconnecting | null>(null);
  protected readonly confirming = signal(false);
  /** The connector the add dialog is for, and whether it is open. */
  protected readonly adding = signal<ConnectorInfo | null>(null);
  protected readonly addOpen = signal(false);

  /** Where a connector can be used: everywhere, or only in one of the person's places. */
  // The button shows only "Change": the row's line says where it is used, in the server's words.
  protected readonly places = computed((): readonly GlzMenuOption[] => [
    { value: EVERYWHERE, label: this.copy.everywhere, short: this.copy.change },
    ...this.projects.projects().map(({ project }) => ({
      value: project.id,
      label: $localize`:@@connectors.onlyIn:Only in ${project.name}:PLACE:`,
      short: this.copy.change,
    })),
  ]);
  /** Whether there is somewhere to choose: with only their Chats, a person sees no places. */
  private readonly choosing = computed(() =>
    this.projects.projects().some(({ project }) => project.kind === 'project'),
  );
  /** The name of the project the page is for, once it is known. */
  protected readonly projectName = computed(() => {
    const id = this.project();
    const found = this.projects.projects().find((it) => it.project.id === id);
    return found?.project.name ?? this.copy.thisProject;
  });
  /** For a project, where what is connected from here is used. */
  protected readonly here = computed(() => {
    const name = this.projectName();
    return this.project() === null
      ? null
      : $localize`:@@connectors.here:Apps you connect here are used only in ${name}:PROJECT:.`;
  });
  /** Whether the server lists any connector here. */
  protected readonly offered = computed(() => (this.service.catalog()?.connectors.length ?? 0) > 0);
  /** Why the list failed, once it had listed some: a refusal before is the page's to say. */
  protected readonly problem = computed(() => (this.offered() ? this.service.problem() : null));
  /** The cards with connectors in them: those connected first, then each category. */
  protected readonly sections = computed((): readonly Section[] => {
    const catalog = this.service.catalog();
    if (catalog === null) {
      return [];
    }
    const query = this.query().trim().toLocaleLowerCase();
    const matches = (it: ConnectorInfo): boolean =>
      query === '' ||
      it.name.toLocaleLowerCase().includes(query) ||
      it.description.toLocaleLowerCase().includes(query);
    const found = catalog.connectors.filter(matches);
    const mine = catalog.connectors.filter((it) => it.status !== 'available');
    const here = catalog.here ?? mine;
    const sections: Section[] = [
      {
        id: 'connectors-connected-heading',
        testId: 'connectors-connected',
        heading: this.connectedHeading(),
        rows: here.filter(matches).map((it) => this.row(it)),
      },
      // For a project, those connected but used elsewhere, which its menu can bring here.
      {
        id: 'connectors-elsewhere-heading',
        testId: 'connectors-elsewhere',
        heading: this.copy.elsewhere,
        rows: mine
          .filter((it) => catalog.here !== null && !here.some((kept) => kept.id === it.id))
          .filter(matches)
          .map((it) => this.row(it)),
      },
    ];
    for (const category of catalog.categories) {
      sections.push({
        id: `connectors-${category.id}-heading`,
        testId: `connectors-${category.id}`,
        heading: category.name,
        rows: found
          .filter((it) => it.categories[0] === category.id && it.status === 'available')
          .map((it) => this.row(it)),
      });
    }
    return sections.filter((section) => section.rows.length > 0);
  });
  /** What the search says when it finds nothing; empty otherwise, its live region kept. */
  protected readonly found = computed(() =>
    this.offered() && this.sections().length === 0 ? this.copy.noMatch : '',
  );
  protected readonly disconnectHeading = computed(() => {
    const name = this.disconnecting()?.called ?? '';
    return $localize`:@@connectors.disconnectHeading:Disconnect ${name}:NAME:?`;
  });

  constructor() {
    effect(() => {
      const project = this.project();
      untracked(() => void this.service.load(project));
    });
    inject(DestroyRef).onDestroy(() => {
      for (const stop of this.stops.values()) {
        stop.abort();
      }
    });
  }

  protected search(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
    }
  }

  /** Uses `row`'s connector where `place` says from now on: everywhere, or one project. */
  protected move(row: Row, place: string | null): void {
    if (place === null || place === row.place) {
      return;
    }
    const only = place === EVERYWHERE ? null : place;
    void this.busyWith(row.id, () => this.service.place(row.id, row.account, only));
  }

  /** What the row's button does: stops waiting, disconnects, or connects (again, where it is). */
  protected act(row: Row): void {
    if (row.busy) {
      return;
    }
    if (row.waiting) {
      this.stops.get(row.id)?.abort();
    } else if (row.connected) {
      this.ask(row);
    } else if (row.again) {
      void this.connect(row.id, row.name, row.account, row.only);
    } else {
      void this.connect(row.id, row.name);
    }
  }

  /** Asks before disconnecting `target`, an account of a connector. */
  protected ask(target: Disconnecting): void {
    this.disconnecting.set(target);
    this.confirming.set(true);
  }

  /** Opens the dialog that adds another account of `row`'s connector. */
  protected addTo(row: Row): void {
    const found = this.service.catalog()?.connectors.find((it) => it.id === row.id) ?? null;
    // For a project, the dialog offers every account of it, not only the one used here.
    this.adding.set(found);
    this.addOpen.set(true);
  }

  /** Does what the dialog chose: uses an account connected already, or signs in to another. */
  protected added(chosen: AccountChosen): void {
    const connector = this.adding();
    if (connector === null) {
      return;
    }
    const { id, name } = connector;
    if (chosen.existing) {
      void this.busyWith(id, () => this.service.place(id, chosen.account, chosen.only));
    } else {
      void this.connect(id, name, chosen.account, chosen.only);
    }
  }

  /** Does what was picked from the menu of `account`, beneath `row`. */
  protected picked(row: Row, account: ConnectorAccount, action: AccountAction): void {
    const name = account.name ?? null;
    if (action.kind === 'place') {
      const only = action.place === EVERYWHERE ? null : action.place;
      void this.busyWith(row.id, () => this.service.place(row.id, name, only));
    } else if (action.kind === 'again') {
      void this.connect(row.id, row.name, name, account.only ?? null);
    } else {
      const called = name === null ? row.name : `${row.name} (${name})`;
      this.ask({ id: row.id, name: row.name, account: name, called });
    }
  }

  protected async decided(yes: boolean): Promise<void> {
    const target = this.disconnecting();
    if (!yes || target === null) {
      return;
    }
    await this.busyWith(target.id, async () => {
      await this.service.disconnect(target.id, target.account);
      this.moved(target.id, this.said(target.called, this.copy.disconnected));
    });
  }

  /**
   * Connects the account `account` of the connector `id`, which the page calls `name`, used
   * only in `only`, by default where the page is. A new tab opens at once, while the click is
   * handled, so no browser blocks it: it goes to the sign-in once the server gives one, and
   * closes when the connector needs none, or the connecting failed or was cancelled first. The
   * row waits until the server says the person signed in there, or they cancel.
   */
  private async connect(
    id: string,
    name: string,
    account: string | null = null,
    only?: ProjectId | null,
  ): Promise<void> {
    this.stops.get(id)?.abort();
    const stop = new AbortController();
    this.stops.set(id, stop);
    const tab = openedTab(this.document);
    let shown = false;
    await this.busyWith(
      id,
      async () => {
        try {
          const link = await this.service.connect(id, account, only);
          if (link === null || stop.signal.aborted) {
            if (link === null) {
              this.moved(id, this.said(name, this.copy.connected));
            }
            return;
          }
          shown = sentTo(tab, link.url);
          this.waiting.update((waiting) => setTo(waiting, id, { link: link.url, opened: shown }));
          void this.announcer.announce(this.waitingLine(name, shown));
          if (await this.service.finish(link.flow, stop.signal)) {
            this.moved(id, this.said(name, this.copy.connected));
          }
        } finally {
          if (!shown) {
            tab?.close();
          }
          this.waiting.update((waiting) => setTo(waiting, id, null));
          if (this.stops.get(id) === stop) {
            this.stops.delete(id);
          }
        }
      },
      stop.signal,
    );
  }

  /**
   * Runs `work` on the connector `id`, its row busy meanwhile, and says why it failed, unless
   * `cancelled` says the person stopped it.
   */
  private async busyWith(
    id: string,
    work: () => Promise<void>,
    cancelled?: AbortSignal,
  ): Promise<void> {
    this.problems.update((problems) => setTo(problems, id, null));
    this.working.update((working) => setTo(working, id, true));
    try {
      await work();
    } catch (error: unknown) {
      if (cancelled?.aborted !== true) {
        this.problems.update((problems) => setTo(problems, id, connectionProblem(error)));
      }
    } finally {
      this.working.update((working) => setTo(working, id, null));
    }
  }

  /**
   * Says that the connector `id` moved to another card, as `said`, and, once it is drawn there,
   * gives its button the focus its old one took with it, unless the person has moved on.
   */
  private moved(id: string, said: string): void {
    void this.announcer.announce(said);
    afterNextRender(
      () => {
        const focused = this.document.activeElement;
        if (focused !== null && focused !== this.document.body) {
          return;
        }
        const button = this.host.nativeElement.querySelector<HTMLElement>(
          `[data-connector="${id}"] [data-testid="connector-action"]`,
        );
        button?.focus();
      },
      { injector: this.injector },
    );
  }

  /** The connected card's heading: for a project, what its threads use. */
  private connectedHeading(): string {
    const name = this.projectName();
    return this.project() === null
      ? this.copy.connected
      : $localize`:@@connectors.usedIn:Used in ${name}:PROJECT:`;
  }

  /** What is said of the connector `name` now in the state `state`. */
  private said(name: string, state: string): string {
    return $localize`:@@connectors.said:${name}:NAME:, ${state}:STATE:`;
  }

  /** What a row waiting for its sign-in says, as the tab for it opened or not. */
  private waitingLine(name: string, opened: boolean): string {
    return opened
      ? $localize`:@@connectors.waiting:Finish signing in to ${name}:NAME: in the new tab.`
      : $localize`:@@connectors.openToFinish:To finish, open the sign-in for ${name}:NAME:.`;
  }

  /** `connector` as a row: what it is, what it is doing, and what its button does. */
  private row(connector: ConnectorInfo): Row {
    const { id, name, status } = connector;
    const connected = status === 'connected';
    const again = status === 'sign_in_again';
    const waited = this.waiting().get(id);
    const busy = this.working().has(id) && waited === undefined;
    const problem = this.problems().get(id) ?? null;
    const all = connector.accounts ?? [];
    const [first] = all;
    const several = all.length > 1;
    const account = first?.name ?? null;
    const called = account === null ? name : `${name} (${account})`;
    const base = {
      id,
      name: account === null || several ? name : `${name}: ${account}`,
      connected,
      again,
      busy,
      problem,
      waiting: false,
      link: null,
      linkText: '',
      disconnectLabel: $localize`:@@connectors.disconnectLabel:Disconnect ${called}:NAME:`,
      place:
        status === 'available' || several || !this.choosing() ? null : (first?.only ?? EVERYWHERE),
      placeLabel: $localize`:@@connectors.placeLabel:Where ${called}:NAME: is used`,
      account,
      only: first?.only ?? null,
      called,
      accounts: several ? all.map((it) => ({ account: it, place: it.only ?? EVERYWHERE })) : [],
      listed: several,
      tile: name,
      acts: !several,
      adds: status !== 'available',
      addLabel: $localize`:@@connectors.addLabel:Add another ${name}:NAME: account`,
      note: connector.replaced ?? null,
      noted: (connector.replaced ?? null) !== null,
    };
    // A sign-in waiting shows first: a connected one's second account's, or one signed in again.
    if (waited !== undefined) {
      return {
        ...base,
        waiting: true,
        acts: true,
        detail: this.waitingLine(name, waited.opened),
        link: waited.link,
        linkText: waited.opened ? this.copy.openAgain : this.copy.openSignIn,
        action: this.copy.cancel,
        actionLabel: $localize`:@@connectors.cancelLabel:Cancel connecting ${name}:NAME:`,
        variant: 'secondary',
      };
    }
    if (several) {
      const count = all.length;
      return {
        ...base,
        again: false,
        detail: plural(
          $localize`:@@connectors.accounts:{${count}:COUNT:, plural, one {# account} other {# accounts}}`,
          this.locale,
        ),
        action: '',
        actionLabel: '',
        variant: 'ghost',
      };
    }
    if (connected) {
      return {
        ...base,
        detail: first?.used ?? this.copy.connected,
        action: busy ? this.copy.disconnecting : this.copy.disconnect,
        actionLabel: base.disconnectLabel,
        variant: 'ghost',
      };
    }
    return {
      ...base,
      detail: again ? this.copy.signInAgain : connector.description,
      action: busy ? this.copy.connecting : again ? this.copy.again : this.copy.connect,
      actionLabel: $localize`:@@connectors.connectLabel:Connect ${name}:NAME:`,
      variant: 'secondary',
    };
  }
}
