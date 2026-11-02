import { LiveAnnouncer } from '@angular/cdk/a11y';
import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
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
import { LoaderCircle, Search } from 'lucide';

import { connectionProblem, openedTab, sentTo } from '../../lib/connections';
import { ConnectorsService } from '../../services/connectors.service';
import { ServiceRowComponent } from '../service-row/service-row.component';

/** A connector as a row of the page shows it. */
interface Row {
  readonly id: string;
  readonly name: string;
  /** Whether it is connected, ready to use. */
  readonly connected: boolean;
  /** Whether it waits for the person to sign in to it in the browser. */
  readonly waiting: boolean;
  /** Whether it is being connected or disconnected. */
  readonly busy: boolean;
  /** The line under its name. */
  readonly detail: string;
  /** Why the last thing done with it failed, in the server's words. */
  readonly problem: string | null;
  /** Its button's word, its name for screen readers, and how much it stands out. */
  readonly action: string;
  readonly actionLabel: string;
  readonly variant: GlzButtonVariant;
}

/** A card of the page: its heading, and its connectors. */
interface Section {
  readonly id: string;
  readonly testId: string;
  readonly heading: string;
  readonly rows: readonly Row[];
}

/** `set` with `key` in it, or out of it. */
function toggled(set: ReadonlySet<string>, key: string, present: boolean): ReadonlySet<string> {
  const next = new Set(set);
  if (present) {
    next.add(key);
  } else {
    next.delete(key);
  }
  return next;
}

/** `map` with `key` set to `value`, or without it when `value` is null. */
function including(
  map: ReadonlyMap<string, string>,
  key: string,
  value: string | null,
): ReadonlyMap<string, string> {
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
 * and waits until the server says it is done. Every word about a connector is the server's.
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
    cancel: $localize`:@@connectors.cancel:Cancel`,
    disconnect: $localize`:@@connectors.disconnect:Disconnect`,
    disconnecting: $localize`:@@connectors.disconnecting:Disconnecting…`,
    disconnected: $localize`:@@connectors.disconnected:Disconnected`,
    disconnectText: $localize`:@@connectors.disconnectText:glave forgets your sign-in and stops using it for you. You can connect it again at any time.`,
    noMatch: $localize`:@@connectors.noMatch:No app matches that search.`,
  } as const;
  protected readonly icons = { waiting: LoaderCircle, search: Search } as const;
  private readonly service = inject(ConnectorsService);
  private readonly document = inject(DOCUMENT);
  private readonly announcer = inject(LiveAnnouncer);
  /** What the person searches the connectors for. */
  protected readonly query = signal('');
  /** The connectors waiting for their sign-ins, and what stops each. */
  private readonly stops = new Map<string, AbortController>();
  private readonly waiting = signal<ReadonlySet<string>>(new Set());
  /** The connectors being connected or disconnected. */
  private readonly working = signal<ReadonlySet<string>>(new Set());
  /** Why the last thing done with a connector failed, by its id. */
  private readonly problems = signal<ReadonlyMap<string, string>>(new Map());
  /** The connector Disconnect asks about. */
  private readonly disconnecting = signal<Row | null>(null);
  protected readonly confirming = signal(false);

  protected readonly problem = this.service.problem;
  /** Whether the server lists any connector here. */
  protected readonly offered = computed(() => (this.service.catalog()?.connectors.length ?? 0) > 0);
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
    const mine = found.filter((it) => it.status !== 'available');
    const sections: Section[] = [
      {
        id: 'connectors-connected-heading',
        testId: 'connectors-connected',
        heading: this.copy.connected,
        rows: mine.map((it) => this.row(it)),
      },
    ];
    for (const category of catalog.categories) {
      sections.push({
        id: `connectors-${category.id}-heading`,
        testId: `connectors-${category.id}`,
        heading: category.name,
        rows: found
          .filter((it) => it.category === category.id && it.status === 'available')
          .map((it) => this.row(it)),
      });
    }
    return sections.filter((section) => section.rows.length > 0);
  });
  /** Whether a search finds nothing. */
  protected readonly nothingFound = computed(() => this.offered() && this.sections().length === 0);
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
      this.disconnecting.set(row);
      this.confirming.set(true);
    } else if (row.waiting) {
      this.stops.get(row.id)?.abort();
    } else {
      void this.connect(row);
    }
  }

  protected async decided(yes: boolean): Promise<void> {
    const row = this.disconnecting();
    if (!yes || row === null) {
      return;
    }
    await this.busyWith(row.id, async () => {
      await this.service.disconnect(row.id);
      void this.announcer.announce(this.said(row.name, this.copy.disconnected));
    });
  }

  /**
   * Connects `row`'s connector. A new tab opens at once, while the click is handled, so no
   * browser blocks it: it goes to the sign-in once the server gives one, and closes when the
   * connector needs none. The row waits until the server says the person signed in there, or
   * they cancel.
   */
  private async connect(row: Row): Promise<void> {
    const { id, name } = row;
    const stop = new AbortController();
    this.stops.set(id, stop);
    const tab = openedTab(this.document);
    await this.busyWith(id, async () => {
      const link = await this.service.connect(id);
      if (link === null) {
        tab?.close();
        void this.announcer.announce(this.said(name, this.copy.connected));
        return;
      }
      const shown = sentTo(tab, link.url);
      if (!shown) {
        tab?.close();
      }
      this.waiting.update((waiting) => toggled(waiting, id, true));
      void this.announcer.announce(this.waitingLine(name));
      try {
        if (await this.service.finish(link.flow, stop.signal)) {
          void this.announcer.announce(this.said(name, this.copy.connected));
        }
      } finally {
        this.waiting.update((waiting) => toggled(waiting, id, false));
      }
    }).finally(() => {
      if (this.stops.get(id) === stop) {
        this.stops.delete(id);
      }
    });
  }

  /** Runs `work` on the connector `id`, its row busy meanwhile, and says why it failed. */
  private async busyWith(id: string, work: () => Promise<void>): Promise<void> {
    this.problems.update((problems) => including(problems, id, null));
    this.working.update((working) => toggled(working, id, true));
    try {
      await work();
    } catch (error: unknown) {
      if (!(this.stops.get(id)?.signal.aborted ?? false)) {
        this.problems.update((problems) => including(problems, id, connectionProblem(error)));
      }
    } finally {
      this.working.update((working) => toggled(working, id, false));
    }
  }

  /** What is said of the connector `name` now in the state `state`. */
  private said(name: string, state: string): string {
    return $localize`:@@connectors.said:${name}:NAME:, ${state}:STATE:`;
  }

  /** What a row waiting for its sign-in says. */
  private waitingLine(name: string): string {
    return $localize`:@@connectors.waiting:Finish signing in to ${name}:NAME: in the new tab.`;
  }

  /** `connector` as a row: what it is, what it is doing, and what its button does. */
  private row(connector: ConnectorInfo): Row {
    const { id, name, status } = connector;
    const connected = status === 'connected';
    const waiting = this.waiting().has(id);
    const busy = this.working().has(id) && !waiting;
    const problem = this.problems().get(id) ?? null;
    const base = { id, name, connected, waiting, busy, problem };
    if (connected) {
      return {
        ...base,
        detail: this.copy.connected,
        action: busy ? this.copy.disconnecting : this.copy.disconnect,
        actionLabel: $localize`:@@connectors.disconnectLabel:Disconnect ${name}:NAME:`,
        variant: 'ghost',
      };
    }
    if (waiting) {
      return {
        ...base,
        detail: this.waitingLine(name),
        action: this.copy.cancel,
        actionLabel: $localize`:@@connectors.cancelLabel:Cancel connecting ${name}:NAME:`,
        variant: 'secondary',
      };
    }
    return {
      ...base,
      detail: status === 'sign_in_again' ? this.copy.signInAgain : connector.description,
      action: busy ? this.copy.connecting : this.copy.connect,
      actionLabel: $localize`:@@connectors.connectLabel:Connect ${name}:NAME:`,
      variant: 'secondary',
    };
  }
}
