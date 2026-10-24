import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import type { ConnectionInfo } from '@glave/client';
import {
  type GlzButtonVariant,
  GlzButtonComponent,
  GlzConfirmDialogComponent,
  GlzEmptyStateComponent,
  GlzIconComponent,
} from '@glave/glaze';
import { ExternalLink, LoaderCircle, Plug } from 'lucide';

import { connectionProblem, serviceHost, serviceName } from '../../lib/connections';
import { ConnectionsService } from '../../services/connections.service';
import { PageHeaderComponent } from '../page-header/page-header.component';
import { ServiceRowComponent } from '../service-row/service-row.component';

/** A service as a row of the page shows it. */
interface Row {
  /** Its address, which names it to the server. */
  readonly url: string;
  readonly name: string;
  /** Whether it is signed in to, ready to use. */
  readonly ready: boolean;
  /** Whether it waits for the person to sign in to it in the new tab. */
  readonly waiting: boolean;
  /** The line under its name. */
  readonly detail: string;
  /** The sign-in's page, to open again while it waits, once the server gave it. */
  readonly link: string | null;
  /** Why the last thing done with it failed, in the server's words. */
  readonly problem: string | null;
  /** Its button's word, its name for screen readers, and how much it stands out. */
  readonly action: string;
  readonly actionLabel: string;
  readonly variant: GlzButtonVariant;
}

/** A card of the page: its heading, and its services. */
interface Section {
  readonly id: string;
  readonly testId: string;
  readonly heading: string;
  readonly rows: readonly Row[];
}

/** `map` without `key`. */
function without<T>(map: ReadonlyMap<string, T>, key: string): ReadonlyMap<string, T> {
  const rest = new Map(map);
  rest.delete(key);
  return rest;
}

/** `map` with `key` set to `value`. */
function including<T>(map: ReadonlyMap<string, T>, key: string, value: T): ReadonlyMap<string, T> {
  return new Map(map).set(key, value);
}

/** Sends `tab` to `url`; whether it could be, as one the person closed cannot. */
function sent(tab: Window | null, url: string): boolean {
  try {
    tab?.location.replace(url);
    return tab !== null && !tab.closed;
  } catch {
    return false;
  }
}

/**
 * Connections: the services glave can use for the person, as the server lists them. Those
 * signed in to are ready to use, each with Disconnect, which asks first; the person's services
 * that ask them to sign in are available, each with Connect, which opens the service's own
 * sign-in in a new tab and waits until the server says it is done. Every refusal is told in the
 * server's words. More sections (glave's starter list, a company's) join the same cards.
 */
@Component({
  selector: 'app-connections',
  imports: [
    PageHeaderComponent,
    ServiceRowComponent,
    GlzButtonComponent,
    GlzConfirmDialogComponent,
    GlzEmptyStateComponent,
    GlzIconComponent,
  ],
  templateUrl: './connections.component.html',
  styleUrl: './connections.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConnectionsComponent {
  protected readonly copy = {
    heading: $localize`:@@connections.heading:Connections`,
    intro: $localize`:@@connections.intro:Services glave can use for you. You sign in to each one yourself, and can disconnect it at any time.`,
    loading: $localize`:@@connections.loading:Looking for your services…`,
    ready: $localize`:@@connections.ready:Ready to use`,
    available: $localize`:@@connections.available:Available`,
    connected: $localize`:@@connections.connected:Connected`,
    expired: $localize`:@@connections.expired:Your sign-in expired`,
    connect: $localize`:@@connections.connect:Connect`,
    cancel: $localize`:@@connections.cancel:Cancel`,
    disconnect: $localize`:@@connections.disconnect:Disconnect`,
    openAgain: $localize`:@@connections.openAgain:Open it again`,
    newTab: $localize`:@@connections.newTab:in a new tab`,
    disconnectText: $localize`:@@connections.disconnectText:glave forgets your sign-in and stops using it for you. You can connect it again at any time.`,
    emptyHeading: $localize`:@@connections.emptyHeading:Nothing to connect yet`,
    emptyText: $localize`:@@connections.emptyText:Services that ask you to sign in show up here.`,
  } as const;
  protected readonly icons = { waiting: LoaderCircle, open: ExternalLink, empty: Plug } as const;
  private readonly service = inject(ConnectionsService);
  private readonly document = inject(DOCUMENT);
  /** The sign-ins waiting, by the service's address: the page to open, once the server gave it. */
  private readonly waiting = signal<ReadonlyMap<string, string | null>>(new Map());
  /** What stops each sign-in waiting, by the service's address. */
  private readonly stops = new Map<string, AbortController>();
  /** Why the last thing done with a service failed, by its address. */
  private readonly problems = signal<ReadonlyMap<string, string>>(new Map());
  /** The service Disconnect asks about. */
  private readonly disconnecting = signal<Row | null>(null);
  protected readonly confirming = signal(false);

  protected readonly problem = this.service.problem;
  /** Whether the services are still being listed for the first time. */
  protected readonly loading = computed(
    () => this.service.connections() === null && this.problem() === null,
  );
  private readonly rows = computed(() =>
    (this.service.connections() ?? []).map((connection) => this.row(connection)),
  );
  /** The cards with services in them: those ready to use first, then those to connect. */
  protected readonly sections = computed(() => {
    const rows = this.rows();
    const sections: readonly Section[] = [
      {
        id: 'connections-ready-heading',
        testId: 'connections-ready',
        heading: this.copy.ready,
        rows: rows.filter((row) => row.ready),
      },
      {
        id: 'connections-available-heading',
        testId: 'connections-available',
        heading: this.copy.available,
        rows: rows.filter((row) => !row.ready),
      },
    ];
    return sections.filter((section) => section.rows.length > 0);
  });
  /** Whether the server listed no service at all. */
  protected readonly empty = computed(() => this.service.connections()?.length === 0);
  protected readonly disconnectHeading = computed(() => {
    const name = this.disconnecting()?.name ?? '';
    return $localize`:@@connections.disconnectHeading:Disconnect ${name}:NAME:?`;
  });

  constructor() {
    void this.service.load();
    inject(DestroyRef).onDestroy(() => {
      for (const stop of this.stops.values()) {
        stop.abort();
      }
    });
  }

  /** What the row's button does: disconnects, stops waiting, or connects. */
  protected act(row: Row): void {
    if (row.ready) {
      this.disconnecting.set(row);
      this.confirming.set(true);
    } else if (row.waiting) {
      this.cancel(row.url);
    } else {
      void this.connect(row.url);
    }
  }

  protected async decided(yes: boolean): Promise<void> {
    const row = this.disconnecting();
    if (!yes || row === null) {
      return;
    }
    this.problems.update((problems) => without(problems, row.url));
    try {
      await this.service.remove(row.url);
    } catch (error: unknown) {
      this.problems.update((problems) => including(problems, row.url, connectionProblem(error)));
    }
  }

  /**
   * Signs in to the service at `url`: a new tab opens at once, while the click is handled, so
   * no browser blocks it, and goes to the service's own sign-in once the server gives it; the
   * row waits until the server says the person signed in there, or they cancel.
   */
  private async connect(url: string): Promise<void> {
    this.stops.get(url)?.abort();
    const stop = new AbortController();
    this.stops.set(url, stop);
    this.problems.update((problems) => without(problems, url));
    this.waiting.update((waiting) => including(waiting, url, null));
    const tab = this.opened();
    let shown = false;
    try {
      const link = await this.service.start(url);
      if (stop.signal.aborted) {
        tab?.close();
        return;
      }
      shown = sent(tab, link.url);
      this.waiting.update((waiting) => including(waiting, url, link.url));
      await this.service.finish(link.flow, stop.signal);
    } catch (error: unknown) {
      if (!shown) {
        tab?.close();
      }
      if (!stop.signal.aborted) {
        this.problems.update((problems) => including(problems, url, connectionProblem(error)));
      }
    } finally {
      if (this.stops.get(url) === stop) {
        this.stops.delete(url);
        this.waiting.update((waiting) => without(waiting, url));
      }
    }
  }

  private cancel(url: string): void {
    this.stops.get(url)?.abort();
    this.stops.delete(url);
    this.waiting.update((waiting) => without(waiting, url));
  }

  /** A new tab, which cannot reach this page; none where the browser opens none. */
  private opened(): Window | null {
    const tab = this.document.defaultView?.open('', '_blank') ?? null;
    if (tab !== null) {
      tab.opener = null;
    }
    return tab;
  }

  /** `connection` as a row: what it is, what it is doing, and what its button does. */
  private row(connection: ConnectionInfo): Row {
    const { url, status } = connection;
    const name = serviceName(connection);
    const ready = status === 'signed_in';
    const waiting = !ready && this.waiting().has(url);
    const base = { url, name, ready, waiting, link: this.waiting().get(url) ?? null };
    const problem = this.problems().get(url) ?? null;
    if (ready) {
      return {
        ...base,
        detail: this.copy.connected,
        problem,
        action: this.copy.disconnect,
        actionLabel: $localize`:@@connections.disconnectLabel:Disconnect ${name}:NAME:`,
        variant: 'ghost',
      };
    }
    if (waiting) {
      return {
        ...base,
        detail: $localize`:@@connections.waiting:Finish signing in to ${name}:NAME: in the new tab.`,
        problem: null,
        action: this.copy.cancel,
        actionLabel: $localize`:@@connections.cancelLabel:Cancel signing in to ${name}:NAME:`,
        variant: 'secondary',
      };
    }
    return {
      ...base,
      detail: status === 'expired' ? this.copy.expired : serviceHost(connection),
      problem,
      action: this.copy.connect,
      actionLabel: $localize`:@@connections.connectLabel:Connect ${name}:NAME:`,
      variant: 'secondary',
    };
  }
}
