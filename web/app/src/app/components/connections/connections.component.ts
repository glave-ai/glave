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

/** A sign-in waiting for the person in the browser. */
interface Waiting {
  /** The service's own sign-in page, once the server gave it. */
  readonly link: string | null;
  /** Whether a new tab is open at it: none where the browser opened none, or it was closed. */
  readonly opened: boolean;
}

/** A service as a row of the page shows it. */
interface Row {
  /** Its address, which names it to the server. */
  readonly url: string;
  readonly name: string;
  /** Whether it is signed in to, ready to use. */
  readonly ready: boolean;
  /** Whether it waits for the person to sign in to it in the browser. */
  readonly waiting: boolean;
  /** Whether its sign-in is being removed. */
  readonly busy: boolean;
  /** The line under its name. */
  readonly detail: string;
  /** How often it was called this month, in the server's words. */
  readonly usage: string | null;
  /** The sign-in's page, to open while it waits, once the server gave it, and the link's word. */
  readonly link: string | null;
  readonly linkText: string;
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

/** Sends `tab` to `url`; whether it is open there, as one the person closed is not. */
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
    disconnected: $localize`:@@connections.disconnected:Disconnected`,
    expired: $localize`:@@connections.expired:Your sign-in expired`,
    connect: $localize`:@@connections.connect:Connect`,
    cancel: $localize`:@@connections.cancel:Cancel`,
    disconnect: $localize`:@@connections.disconnect:Disconnect`,
    disconnecting: $localize`:@@connections.disconnecting:Disconnecting…`,
    openAgain: $localize`:@@connections.openAgain:Open it again`,
    openSignIn: $localize`:@@connections.openSignIn:Open the sign-in`,
    newTab: $localize`:@@connections.newTab:in a new tab`,
    disconnectText: $localize`:@@connections.disconnectText:glave forgets your sign-in and stops using it for you. You can connect it again at any time.`,
    emptyHeading: $localize`:@@connections.emptyHeading:Nothing to connect yet`,
    emptyText: $localize`:@@connections.emptyText:Services that ask you to sign in show up here.`,
  } as const;
  protected readonly icons = { waiting: LoaderCircle, open: ExternalLink, empty: Plug } as const;
  private readonly service = inject(ConnectionsService);
  private readonly document = inject(DOCUMENT);
  private readonly announcer = inject(LiveAnnouncer);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly injector = inject(Injector);
  /** The sign-ins waiting, by the service's address. */
  private readonly waiting = signal<ReadonlyMap<string, Waiting>>(new Map());
  /** What stops each sign-in waiting, by the service's address. */
  private readonly stops = new Map<string, AbortController>();
  /** The services whose sign-ins are being removed, by address. */
  private readonly removing = signal<ReadonlySet<string>>(new Set());
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
    if (row.busy) {
      return;
    }
    if (row.ready) {
      this.disconnecting.set(row);
      this.confirming.set(true);
    } else if (row.waiting) {
      this.cancel(row.url);
    } else {
      void this.connect(row);
    }
  }

  protected async decided(yes: boolean): Promise<void> {
    const row = this.disconnecting();
    if (!yes || row === null) {
      return;
    }
    const { url, name } = row;
    this.problems.update((problems) => without(problems, url));
    this.removing.update((removing) => new Set(removing).add(url));
    try {
      await this.service.remove(url);
      this.moved(url, this.said(name, this.copy.disconnected));
    } catch (error: unknown) {
      this.problems.update((problems) => including(problems, url, connectionProblem(error)));
    } finally {
      this.removing.update((removing) => {
        const rest = new Set(removing);
        rest.delete(url);
        return rest;
      });
    }
  }

  /**
   * Signs in to `row`'s service: a new tab opens at once, while the click is handled, so no
   * browser blocks it, and goes to the service's own sign-in once the server gives it; the row
   * waits until the server says the person signed in there, or they cancel.
   */
  private async connect(row: Row): Promise<void> {
    const { url, name } = row;
    this.stops.get(url)?.abort();
    const stop = new AbortController();
    this.stops.set(url, stop);
    this.problems.update((problems) => without(problems, url));
    const tab = this.opened();
    this.waiting.update((waiting) => including(waiting, url, { link: null, opened: tab !== null }));
    let shown = false;
    try {
      const link = await this.service.start(url);
      if (stop.signal.aborted) {
        tab?.close();
        return;
      }
      shown = sent(tab, link.url);
      this.waiting.update((waiting) => including(waiting, url, { link: link.url, opened: shown }));
      void this.announcer.announce(this.waitingLine(name, shown));
      if ((await this.service.finish(link.flow, stop.signal)) !== null) {
        this.moved(url, this.said(name, this.copy.connected));
      }
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

  /**
   * A new tab, which cannot reach this page; none where the browser opens none. Once it reaches
   * the sign-in, on another origin, glave's opener policy cuts it off from this page too, and
   * this page from it.
   */
  private opened(): Window | null {
    const tab = this.document.defaultView?.open('', '_blank') ?? null;
    if (tab !== null) {
      tab.opener = null;
    }
    return tab;
  }

  /**
   * Says that the service at `url` moved to another card, as `said`, and, once it is drawn
   * there, gives its button the focus its old one took with it, unless the person has moved on.
   */
  private moved(url: string, said: string): void {
    void this.announcer.announce(said);
    afterNextRender(
      () => {
        const focused = this.document.activeElement;
        if (focused !== null && focused !== this.document.body) {
          return;
        }
        const rows = this.host.nativeElement.querySelectorAll('app-service-row');
        const row = [...rows].find((found) => found.getAttribute('data-url') === url);
        row?.querySelector<HTMLElement>('[data-testid="connection-action"]')?.focus();
      },
      { injector: this.injector },
    );
  }

  /** What is said of the service `name` now in the state `state`, a row's own words. */
  private said(name: string, state: string): string {
    return $localize`:@@connections.said:${name}:NAME:, ${state}:STATE:`;
  }

  /** What a row waiting for its sign-in says, as the tab for it opened or not. */
  private waitingLine(name: string, opened: boolean): string {
    return opened
      ? $localize`:@@connections.waiting:Finish signing in to ${name}:NAME: in the new tab.`
      : $localize`:@@connections.openToFinish:To finish, open the sign-in for ${name}:NAME:.`;
  }

  /** `connection` as a row: what it is, what it is doing, and what its button does. */
  private row(connection: ConnectionInfo): Row {
    const { url, status } = connection;
    const name = serviceName(connection);
    const ready = status === 'signed_in';
    const waited = ready ? undefined : this.waiting().get(url);
    const busy = this.removing().has(url);
    const problem = this.problems().get(url) ?? null;
    const usage = connection.usage?.sentence ?? null;
    const base = { url, name, ready, busy, problem, usage, link: waited?.link ?? null };
    if (ready) {
      return {
        ...base,
        waiting: false,
        detail: this.copy.connected,
        linkText: '',
        action: busy ? this.copy.disconnecting : this.copy.disconnect,
        actionLabel: $localize`:@@connections.disconnectLabel:Disconnect ${name}:NAME:`,
        variant: 'ghost',
      };
    }
    if (waited !== undefined) {
      return {
        ...base,
        waiting: true,
        problem: null,
        detail: this.waitingLine(name, waited.opened),
        linkText: waited.opened ? this.copy.openAgain : this.copy.openSignIn,
        action: this.copy.cancel,
        actionLabel: $localize`:@@connections.cancelLabel:Cancel signing in to ${name}:NAME:`,
        variant: 'secondary',
      };
    }
    return {
      ...base,
      waiting: false,
      detail: status === 'expired' ? this.copy.expired : serviceHost(connection),
      linkText: '',
      action: this.copy.connect,
      actionLabel: $localize`:@@connections.connectLabel:Connect ${name}:NAME: in a new tab`,
      variant: 'secondary',
    };
  }
}
