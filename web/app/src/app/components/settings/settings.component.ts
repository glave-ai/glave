import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzConfirmDialogComponent,
  GlzFormFieldComponent,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { REMOVE_FAILED, accountName, changeQuery, keyHint } from '../../lib/providers';
import { ModelsService } from '../../services/models.service';
import { ProvidersService } from '../../services/providers.service';

import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';
import { ThemeService, isChoice } from '../../services/theme.service';
import { AccountsComponent } from '../accounts/accounts.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

const COPY = {
  heading: 'Settings',
  appearance: 'Appearance',
  theme: 'Theme',
  system: 'System',
  light: 'Light',
  dark: 'Dark',
  models: 'Models',
  noAccounts: 'No model accounts yet.',
  add: 'Add a model account',
  change: 'Change key',
  remove: 'Remove',
  removeHeading: 'Remove this model account?',
  removeText: 'glave forgets its key, and its models are no longer offered.',
  memory: 'Memory',
  memoryText: 'What glave keeps between threads, about you and about each project.',
  memoryOpen: 'Open memory',
  server: 'This server',
  serverUnknown: 'Reading…',
  signOut: 'Sign out of this tab',
  signOutOthers: 'Sign out other browsers',
  signedOutOthers: 'Every other browser is signed out. This tab stays signed in.',
  signOutOthersFailed: 'Other browsers could not be signed out. Try again.',
} as const;

/**
 * Settings: the theme, the model accounts (add, change, remove), the connected accounts git
 * signs in with, memory (its own page), the server, signing this tab or every other browser
 * out.
 */
@Component({
  selector: 'app-settings',
  imports: [
    AccountsComponent,
    PageHeaderComponent,
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
  protected readonly copy = COPY;
  protected readonly themes = [
    { value: 'system', label: COPY.system },
    { value: 'light', label: COPY.light },
    { value: 'dark', label: COPY.dark },
  ] as const;
  private readonly theme = inject(ThemeService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  private readonly info = inject(ServerService).info;
  private readonly providers = inject(ProvidersService);
  private readonly models = inject(ModelsService);
  protected readonly connectLink = '/connect';
  protected readonly memoryLink = '/memory';
  /** The accounts as the list shows them, each with the link that changes its key. */
  protected readonly accounts = computed(() =>
    this.providers.accounts().map((account) => ({
      id: account.id,
      name: accountName(account),
      hint: keyHint(account),
      query: changeQuery(account),
    })),
  );
  protected readonly confirming = signal(false);
  protected readonly problem = signal<string | null>(null);
  /** What signing other browsers out came to, once asked. */
  protected readonly others = signal<string | null>(null);
  private removing: string | null = null;

  protected readonly choice = this.theme.choice;
  protected readonly server = computed(() => {
    const info = this.info();
    return info === null
      ? COPY.serverUnknown
      : `${info.name} ${info.version} · protocol ${info.protocols.join(', ')} · approvals: ${info.approval}`;
  });

  constructor() {
    void inject(ServerService).load();
    void this.providers.load();
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
      this.problem.set(REMOVE_FAILED);
      return;
    }
    // With no model left, the person adds one before anything else.
    if (await this.models.none()) {
      await this.router.navigateByUrl(this.connectLink);
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
      this.others.set(COPY.signedOutOthers);
    } catch {
      this.others.set(COPY.signOutOthersFailed);
    }
  }
}
