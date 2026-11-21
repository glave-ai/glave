import { LiveAnnouncer } from '@angular/cdk/a11y';
import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import type { ConnectorInfo } from '@glave/client';
import { GlzButtonComponent, GlzIconComponent, initials } from '@glave/glaze';
import { LoaderCircle } from 'lucide';

import { connectionProblem, openedTab, sentTo, signInWaiting } from '../../lib/connections';
import type { SuggestionItem } from '../../lib/conversation';
import { ConnectorsService } from '../../services/connectors.service';

/** The card as it shows: offering to connect its connector, or saying it is connected. */
type Shown =
  | {
      readonly kind: 'offer';
      readonly letters: string;
      readonly name: string;
      readonly description: string;
      readonly connect: string;
    }
  | {
      readonly kind: 'connected';
      readonly letters: string;
      /** That it is connected, and as whom where the service says. */
      readonly state: string;
      /** Where it is used, in the server's words. */
      readonly used: string;
    };

/** `connector`, connected, in its line: as whom its account used everywhere is signed in. */
function connectedLine(connector: ConnectorInfo, letters: string): Shown {
  const { name } = connector;
  const accounts = connector.accounts ?? [];
  const account = accounts.find((it) => (it.only ?? null) === null) ?? accounts[0];
  const who = account?.label ?? account?.who ?? null;
  const state =
    who === null
      ? $localize`:@@connectCard.connected:${name}:NAME: connected`
      : $localize`:@@connectCard.connectedAs:${name}:NAME: connected as ${who}:WHO:`;
  return { kind: 'connected', letters, state, used: account?.used ?? '' };
}

/**
 * A connector a search suggested, as a card in the chat: its tile, name and what it reaches, as
 * the server lists it, then the server's sentence, Connect, which connects it everywhere as the
 * Connections page does (in a new tab where it asks for a sign-in, waiting until the server says
 * it is done), and Not now, which puts it off in this tab. Once connected, a line saying so and
 * where it is used. Nothing while the server lists no such connector to connect.
 */
@Component({
  selector: 'app-connect-card',
  imports: [GlzButtonComponent, GlzIconComponent],
  templateUrl: './connect-card.component.html',
  styleUrl: './connect-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConnectCardComponent {
  readonly item = input.required<SuggestionItem>();

  private readonly service = inject(ConnectorsService);
  private readonly document = inject(DOCUMENT);
  private readonly announcer = inject(LiveAnnouncer);
  protected readonly copy = {
    notNow: $localize`:@@connectCard.notNow:Not now`,
    between: '·',
  } as const;
  protected readonly icons = { waiting: LoaderCircle } as const;
  /** Whether it is being connected. */
  protected readonly busy = signal(false);
  /** What its sign-in waiting says, while it waits. */
  protected readonly waiting = signal<string | null>(null);
  /** Why connecting failed last, in the server's words. */
  protected readonly problem = signal<string | null>(null);
  /** What stops its sign-in waiting. */
  private stop: AbortController | null = null;

  /** The connector, as the server lists it now: none until listed, or where it lists none. */
  private readonly connector = computed(() => {
    const id = this.item().connector;
    return this.service.catalog()?.connectors.find((it) => it.id === id) ?? null;
  });
  protected readonly shown = computed((): Shown | null => {
    const connector = this.connector();
    if (connector === null) {
      return null;
    }
    const letters = initials(connector.name);
    if (connector.status !== 'available') {
      return connectedLine(connector, letters);
    }
    if (this.service.notNow().has(connector.id)) {
      return null;
    }
    const { name } = connector;
    return {
      kind: 'offer',
      letters,
      name,
      description: connector.description,
      connect: $localize`:@@connectCard.connect:Connect ${name}:NAME:`,
    };
  });

  constructor() {
    if (this.service.catalog() === null) {
      void this.service.load();
    }
    inject(DestroyRef).onDestroy(() => this.stop?.abort());
  }

  /**
   * Connects it, used everywhere. A new tab opens at once, while the click is handled, so no
   * browser blocks it: it goes to the sign-in once the server gives one, and closes when the
   * connector needs none, or connecting failed first.
   */
  protected async connect(): Promise<void> {
    const connector = this.connector();
    if (connector === null) {
      return;
    }
    this.stop?.abort();
    const stop = new AbortController();
    this.stop = stop;
    const tab = openedTab(this.document);
    let opened = false;
    this.busy.set(true);
    this.problem.set(null);
    try {
      const link = await this.service.connect(connector.id, null, null);
      if (link !== null) {
        opened = sentTo(tab, link.url);
        const waiting = signInWaiting(connector.name, opened);
        this.waiting.set(waiting);
        void this.announcer.announce(waiting);
        await this.service.finish(link.flow, stop.signal);
      }
      const line = this.shown();
      if (line?.kind === 'connected') {
        void this.announcer.announce(line.state);
      }
    } catch (error: unknown) {
      if (!stop.signal.aborted) {
        this.problem.set(connectionProblem(error));
      }
    } finally {
      if (!opened) {
        tab?.close();
      }
      this.waiting.set(null);
      this.busy.set(false);
    }
  }

  /** Puts it off in this tab, a sign-in waiting for it stopped. */
  protected notNow(): void {
    this.stop?.abort();
    this.service.putOffSuggesting(this.item().connector);
  }
}
