import { LiveAnnouncer } from '@angular/cdk/a11y';
import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  Injector,
  afterNextRender,
  computed,
  inject,
  signal,
} from '@angular/core';
import type { ConnectorInfo } from '@glave/client';
import {
  type GlzButtonVariant,
  GlzButtonComponent,
  GlzConfirmDialogComponent,
  GlzIconComponent,
} from '@glave/glaze';
import { ExternalLink, LoaderCircle, Search } from 'lucide';

import { connectionProblem, openedTab, sentTo } from '../../lib/connections';
import { ConnectorsService } from '../../services/connectors.service';
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
}

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
 * with Disconnect, which asks first; then the rest by category, each with Connect, which
 * connects one that asks no one to sign in at once, and otherwise opens its sign-in in a new tab
 * and waits until the server says it is done. Names, descriptions and categories are the
 * server's; the page writes only its own labels for a connector's status.
 */
@Component({
  selector: 'app-connectors',
  imports: [ServiceRowComponent, GlzButtonComponent, GlzConfirmDialogComponent, GlzIconComponent],
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
  } as const;
  protected readonly icons = { waiting: LoaderCircle, search: Search, open: ExternalLink } as const;
  private readonly service = inject(ConnectorsService);
  private readonly document = inject(DOCUMENT);
  private readonly announcer = inject(LiveAnnouncer);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly injector = inject(Injector);
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
  private readonly disconnecting = signal<Row | null>(null);
  protected readonly confirming = signal(false);

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
    const found = catalog.connectors.filter(
      (it) =>
        query === '' ||
        it.name.toLocaleLowerCase().includes(query) ||
        it.description.toLocaleLowerCase().includes(query),
    );
    const sections: Section[] = [
      {
        id: 'connectors-connected-heading',
        testId: 'connectors-connected',
        heading: this.copy.connected,
        rows: found.filter((it) => it.status !== 'available').map((it) => this.row(it)),
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
    const name = this.disconnecting()?.name ?? '';
    return $localize`:@@connectors.disconnectHeading:Disconnect ${name}:NAME:?`;
  });

  constructor() {
    void this.service.load();
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

  /** What the row's button does: disconnects, stops waiting, or connects. */
  protected act(row: Row): void {
    if (row.busy) {
      return;
    }
    if (row.connected) {
      this.ask(row);
    } else if (row.waiting) {
      this.stops.get(row.id)?.abort();
    } else {
      void this.connect(row);
    }
  }

  /** Asks before disconnecting `row`'s connector. */
  protected ask(row: Row): void {
    this.disconnecting.set(row);
    this.confirming.set(true);
  }

  protected async decided(yes: boolean): Promise<void> {
    const row = this.disconnecting();
    if (!yes || row === null) {
      return;
    }
    await this.busyWith(row.id, async () => {
      await this.service.disconnect(row.id);
      this.moved(row.id, this.said(row.name, this.copy.disconnected));
    });
  }

  /**
   * Connects `row`'s connector. A new tab opens at once, while the click is handled, so no
   * browser blocks it: it goes to the sign-in once the server gives one, and closes when the
   * connector needs none, or the connecting failed or was cancelled first. The row waits until
   * the server says the person signed in there, or they cancel.
   */
  private async connect(row: Row): Promise<void> {
    const { id, name } = row;
    this.stops.get(id)?.abort();
    const stop = new AbortController();
    this.stops.set(id, stop);
    const tab = openedTab(this.document);
    let shown = false;
    await this.busyWith(
      id,
      async () => {
        try {
          const link = await this.service.connect(id);
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
    const base = {
      id,
      name,
      connected,
      again,
      busy,
      problem,
      waiting: false,
      link: null,
      linkText: '',
      disconnectLabel: $localize`:@@connectors.disconnectLabel:Disconnect ${name}:NAME:`,
    };
    if (connected) {
      return {
        ...base,
        detail: this.copy.connected,
        action: busy ? this.copy.disconnecting : this.copy.disconnect,
        actionLabel: base.disconnectLabel,
        variant: 'ghost',
      };
    }
    if (waited !== undefined) {
      return {
        ...base,
        waiting: true,
        detail: this.waitingLine(name, waited.opened),
        link: waited.link,
        linkText: waited.opened ? this.copy.openAgain : this.copy.openSignIn,
        action: this.copy.cancel,
        actionLabel: $localize`:@@connectors.cancelLabel:Cancel connecting ${name}:NAME:`,
        variant: 'secondary',
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
