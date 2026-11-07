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
  effect,
  inject,
  input,
  signal,
  untracked,
} from '@angular/core';
import type { ConnectorInfo, ProjectId } from '@glave/client';
import { GlzConfirmDialogComponent, GlzIconComponent, type GlzMenuOption } from '@glave/glaze';
import { Search } from 'lucide';

import { connectionProblem, openedTab, sentTo, signInWaiting } from '../../lib/connections';
import { ConnectorsService } from '../../services/connectors.service';
import { ProjectsService } from '../../services/projects.service';
import {
  type AccountChosen,
  ConnectorAddComponent,
} from '../connector-add/connector-add.component';
import {
  type ConnectorAction,
  ConnectorRowComponent,
  EVERYWHERE,
  type Waiting,
} from '../connector-row/connector-row.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';

/** A connector as its card lists it: the connector, and what the page is doing with it. */
interface Listed {
  readonly connector: ConnectorInfo;
  readonly waiting: Waiting | null;
  readonly working: boolean;
  readonly problem: string | null;
}

/** What Disconnect or Rename asks about: an account of the connector `id`, as its row asked. */
type Asked<K> = Extract<ConnectorAction, { kind: K }> & { readonly id: string };

/** A card of the page: its heading, and its connectors. */
interface Section {
  readonly id: string;
  readonly testId: string;
  readonly heading: string;
  readonly rows: readonly Listed[];
}

/** `map` with `key` set to `value`, or without it when `value` is null. */
function setTo<T>(map: ReadonlyMap<string, T>, key: string, value: T | null): Map<string, T> {
  const next = new Map(map);
  if (value === null) {
    next.delete(key);
    return next;
  }
  return next.set(key, value);
}

/**
 * glave's connectors, on the Connections page: a search over them, those connected first (for a
 * `project`, only those its threads use, and one connected here is used only there), then the
 * rest by category, each a row. It does what the person asks of a row: connects (in a new tab
 * where a sign-in is needed, waiting until the server says it is done), moves, adds an account,
 * renames, or disconnects, which asks first. Names, descriptions and categories are the server's.
 */
@Component({
  selector: 'app-connectors',
  imports: [
    GlzConfirmDialogComponent,
    GlzIconComponent,
    ConnectorRowComponent,
    ConnectorAddComponent,
    RenameDialogComponent,
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
    disconnect: $localize`:@@connectors.disconnect:Disconnect`,
    disconnected: $localize`:@@connectors.disconnected:Disconnected`,
    disconnectText: $localize`:@@connectors.disconnectText:glave stops using it for you and forgets any sign-in to it. You can connect it again at any time.`,
    noMatch: $localize`:@@connectors.noMatch:No app matches that search.`,
    everywhere: $localize`:@@connectors.everywhere:Everywhere, including chats`,
    thisProject: $localize`:@@connectors.thisProject:this project`,
    elsewhere: $localize`:@@connectors.elsewhere:Connected elsewhere`,
  } as const;
  /** The project the page is for, whose threads alone use what is connected from it. */
  readonly project = input<ProjectId | null>(null);
  protected readonly icons = { search: Search } as const;
  private readonly service = inject(ConnectorsService);
  private readonly projects = inject(ProjectsService);
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
  private readonly disconnecting = signal<Asked<'disconnect'> | null>(null);
  protected readonly confirming = signal(false);
  /** The connector the add dialog is for, and whether it is open. */
  protected readonly adding = signal<ConnectorInfo | null>(null);
  protected readonly addOpen = signal(false);
  /** The account the rename dialog is for, by its connector and name, and whether it is open. */
  protected readonly renaming = signal<Asked<'rename'> | null>(null);
  protected readonly renameOpen = signal(false);
  /** What the rename dialog starts from, its heading and its note. */
  protected readonly renameShown = computed(() => {
    const it = this.renaming();
    return { title: it?.called ?? '', heading: it?.heading ?? null, note: it?.note ?? null };
  });

  /** Where a connector can be used: everywhere, or only in one of the person's places. */
  protected readonly places = computed((): readonly GlzMenuOption[] => [
    { value: EVERYWHERE, label: this.copy.everywhere },
    ...this.projects.projects().map(({ project }) => ({
      value: project.id,
      label: $localize`:@@connectors.onlyIn:Only in ${project.name}:PLACE:`,
    })),
  ]);
  /** Whether there is somewhere to choose: with only their Chats, a person sees no places. */
  protected readonly choosing = computed(() =>
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
    // For a project, those connected but used elsewhere, which its menu can bring here.
    const elsewhere =
      catalog.here === null ? [] : mine.filter((it) => !here.some((kept) => kept.id === it.id));
    // For a project, the connected card is what its threads use.
    const name = this.projectName();
    const used =
      this.project() === null
        ? this.copy.connected
        : $localize`:@@connectors.usedIn:Used in ${name}:PROJECT:`;
    const sections = [
      this.section('connected', used, here.filter(matches)),
      this.section('elsewhere', this.copy.elsewhere, elsewhere.filter(matches)),
      ...catalog.categories.map((category) =>
        this.section(
          category.id,
          category.name,
          found.filter((it) => it.categories[0] === category.id && it.status === 'available'),
        ),
      ),
    ];
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

  /** Does what the person asked of `connector` on its row. */
  protected acted(connector: ConnectorInfo, action: ConnectorAction): void {
    const { id, name } = connector;
    switch (action.kind) {
      case 'connect':
        void this.connect(id, name, action.account, action.only);
        break;
      case 'cancel':
        this.stops.get(id)?.abort();
        break;
      case 'place':
        void this.busyWith(id, () => this.service.place(id, action.account, action.only));
        break;
      case 'add':
        // For a project, the dialog offers every account of it, not only the one used here.
        this.adding.set(connector);
        this.addOpen.set(true);
        break;
      case 'rename':
        this.renaming.set({ id, ...action });
        this.renameOpen.set(true);
        break;
      case 'disconnect':
        this.disconnecting.set({ id, ...action });
        this.confirming.set(true);
        break;
    }
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
      void this.connect(id, name, chosen.account, chosen.only, true);
    }
  }

  /** Gives the account the rename dialog is for `name`. */
  protected renamed(name: string): void {
    const renaming = this.renaming();
    if (renaming !== null) {
      const { id, account } = renaming;
      void this.busyWith(id, () => this.service.rename(id, account, name));
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
    another = false,
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
          const link = await this.service.connect(id, account, only, another);
          if (link === null || stop.signal.aborted) {
            if (link === null) {
              this.moved(id, this.said(name, this.copy.connected));
            }
            return;
          }
          shown = sentTo(tab, link.url);
          this.waiting.update((waiting) => setTo(waiting, id, { link: link.url, opened: shown }));
          void this.announcer.announce(signInWaiting(name, shown));
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
        // Its one button: its own, its menu once connected, or with two accounts its Add account.
        const button = ['connector-action', 'connector-menu-trigger', 'connector-add-account']
          .map((it) => `[data-connector="${id}"] [data-testid="${it}"]`)
          .join(', ');
        this.host.nativeElement.querySelector<HTMLElement>(button)?.focus();
      },
      { injector: this.injector },
    );
  }

  /** What is said of the connector `name` now in the state `state`. */
  private said(name: string, state: string): string {
    return $localize`:@@connectors.said:${name}:NAME:, ${state}:STATE:`;
  }

  /** The card `key`, headed `heading`, listing `connectors` with what the page does with each. */
  private section(key: string, heading: string, connectors: readonly ConnectorInfo[]): Section {
    const rows = connectors.map((connector) => ({
      connector,
      waiting: this.waiting().get(connector.id) ?? null,
      working: this.working().has(connector.id),
      problem: this.problems().get(connector.id) ?? null,
    }));
    return { id: `connectors-${key}-heading`, testId: `connectors-${key}`, heading, rows };
  }
}
