import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  signal,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzConfirmDialogComponent,
  GlzFormFieldComponent,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { serviceName } from '../../lib/connections';
import { accountName as hostName } from '../../lib/credentials';
import { accountName, changeQuery, keyHint, refreshProblem } from '../../lib/providers';
import { removeFailed } from '../../lib/text';
import { ConnectionsService } from '../../services/connections.service';
import { CredentialsService } from '../../services/credentials.service';
import { ModelsService } from '../../services/models.service';
import { ProvidersService } from '../../services/providers.service';

import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';
import { ThemeService, isChoice } from '../../services/theme.service';
import { AwakeComponent } from '../awake/awake.component';
import { PasskeysComponent } from '../passkeys/passkeys.component';
import { PageHeaderComponent } from '../page-header/page-header.component';
import { WebSearchComponent } from '../web-search/web-search.component';

/**
 * Settings: the theme, the model accounts (add, change, remove), connections (what is
 * connected, and the page where they are) and memory (its own page), whether glave may search
 * the web, the passkeys that sign the person in where glave is reached from elsewhere, whether
 * their own computer stays awake while glave works, the server, signing this browser or every
 * other out.
 */
@Component({
  selector: 'app-settings',
  imports: [
    AwakeComponent,
    PasskeysComponent,
    PageHeaderComponent,
    WebSearchComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzConfirmDialogComponent,
    GlzFormFieldComponent,
    GlzRadioComponent,
    GlzRadioGroupComponent,
    RouterLink,
  ],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SettingsComponent {
  protected readonly copy = {
    heading: $localize`:@@settings.heading:Settings`,
    appearance: $localize`:@@settings.appearance:Appearance`,
    theme: $localize`:@@settings.theme:Theme`,
    system: $localize`:@@settings.system:System`,
    light: $localize`:@@settings.light:Light`,
    dark: $localize`:@@settings.dark:Dark`,
    models: $localize`:@@settings.models:Models`,
    noAccounts: $localize`:@@settings.noAccounts:No model accounts yet.`,
    add: $localize`:@@settings.add:Add a model account`,
    refresh: $localize`:@@settings.refresh:Refresh models`,
    refreshing: $localize`:@@settings.refreshing:Refreshing…`,
    refreshed: $localize`:@@settings.refreshed:The models are up to date.`,
    change: $localize`:@@settings.change:Change key`,
    remove: $localize`:@@settings.remove:Remove`,
    removeHeading: $localize`:@@settings.removeHeading:Remove this model account?`,
    removeText: $localize`:@@settings.removeText:glave forgets its key, and its models are no longer offered.`,
    connections: $localize`:@@settings.connections:Connections`,
    connectionsText: $localize`:@@settings.connectionsText:Services glave can use for you, each signed in to as you.`,
    connectionsOpen: $localize`:@@settings.connectionsOpen:Open connections`,
    memory: $localize`:@@settings.memory:Memory`,
    memoryText: $localize`:@@settings.memoryText:What glave keeps between threads, about you and about each project.`,
    memoryOpen: $localize`:@@settings.memoryOpen:Open memory`,
    server: $localize`:@@settings.server:This server`,
    serverUnknown: $localize`:@@settings.serverUnknown:Reading…`,
    signOut: $localize`:@@settings.signOut:Sign out of this browser`,
    signOutOthers: $localize`:@@settings.signOutOthers:Sign out other browsers`,
    signedOutOthers: $localize`:@@settings.signedOutOthers:Every other browser is signed out. This one stays signed in.`,
    signOutOthersFailed: $localize`:@@settings.signOutOthersFailed:Other browsers could not be signed out. Try again.`,
  } as const;
  protected readonly themes = [
    { value: 'system', label: this.copy.system },
    { value: 'light', label: this.copy.light },
    { value: 'dark', label: this.copy.dark },
  ] as const;
  private readonly theme = inject(ThemeService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  private readonly info = inject(ServerService).info;
  private readonly providers = inject(ProvidersService);
  private readonly models = inject(ModelsService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly connectLink = '/connect';
  protected readonly memoryLink = '/memory';
  protected readonly connectionsLink = '/settings/connections';
  private readonly hosts = inject(CredentialsService);
  private readonly connections = inject(ConnectionsService);
  /** What the Connections card says: what is connected, by name, else what connections are. */
  protected readonly connected = computed(() => {
    const names = [
      ...this.hosts.accounts().map((account) => hostName(account)),
      ...(this.connections.connections() ?? [])
        .filter((connection) => connection.status === 'signed_in')
        .map((connection) => serviceName(connection)),
    ];
    if (names.length === 0) {
      return this.copy.connectionsText;
    }
    const list = new Intl.ListFormat(this.locale, { type: 'conjunction' }).format(names);
    return names.length === 1
      ? $localize`:@@settings.connectedOne:${list}:NAMES: is connected. Services glave can use for you, each signed in to as you.`
      : $localize`:@@settings.connectedMany:${list}:NAMES: are connected. Services glave can use for you, each signed in to as you.`;
  });
  /** The accounts as the list shows them, each with the link that changes its key. */
  protected readonly accounts = computed(() =>
    this.providers.accounts().map((account) => ({
      id: account.id,
      name: accountName(account),
      hint: keyHint(account, this.locale),
      query: changeQuery(account),
    })),
  );
  protected readonly confirming = signal(false);
  protected readonly problem = signal<string | null>(null);
  /** Whether the models are being listed again. */
  protected readonly refreshing = signal(false);
  /** What refreshing the models came to, once asked. */
  protected readonly refreshed = signal<string | null>(null);
  protected readonly refreshLabel = computed(() =>
    this.refreshing() ? this.copy.refreshing : this.copy.refresh,
  );
  protected readonly hasAccounts = computed(() => this.accounts().length > 0);
  /** Whether passkeys sign the person in: glave is reached from elsewhere. */
  protected readonly passkeys = this.session.passkeys;
  /** What signing other browsers out came to, once asked. */
  protected readonly others = signal<string | null>(null);
  private removing: string | null = null;

  protected readonly choice = this.theme.choice;
  protected readonly server = computed(() => {
    const info = this.info();
    if (info === null) {
      return this.copy.serverUnknown;
    }
    const protocols = info.protocols.join(', ');
    return $localize`:@@settings.serverInfo:${info.name}:NAME: ${info.version}:VERSION: · protocol ${protocols}:PROTOCOLS:`;
  });

  constructor() {
    void inject(ServerService).load();
    void this.providers.load();
    void this.hosts.load();
    void this.connections.load();
  }

  protected ask(id: string): void {
    this.removing = id;
    this.confirming.set(true);
  }

  protected async decided(yes: boolean): Promise<void> {
    const id = this.removing;
    this.removing = null;
    if (!yes || id === null) {
      return;
    }
    this.problem.set(null);
    try {
      await this.providers.remove(id);
    } catch {
      this.problem.set(removeFailed());
      return;
    }
    // With no model left, the person adds one before anything else.
    if (await this.models.none()) {
      await this.router.navigateByUrl(this.connectLink);
    }
  }

  protected async refresh(): Promise<void> {
    this.refreshing.set(true);
    this.refreshed.set(null);
    this.problem.set(null);
    try {
      await this.providers.refresh();
      this.refreshed.set(this.copy.refreshed);
    } catch (error: unknown) {
      this.problem.set(refreshProblem(error));
    } finally {
      this.refreshing.set(false);
    }
  }

  protected pick(choice: string | null): void {
    if (isChoice(choice)) {
      this.theme.set(choice);
    }
  }

  protected async signOut(): Promise<void> {
    await this.session.signOut();
    await this.router.navigateByUrl('/sign-in');
  }

  protected async signOutOthers(): Promise<void> {
    this.others.set(null);
    try {
      await this.session.signOutOthers();
      this.others.set(this.copy.signedOutOthers);
    } catch {
      this.others.set(this.copy.signOutOthersFailed);
    }
  }
}
