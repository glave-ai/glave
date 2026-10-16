import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
  type WritableSignal,
} from '@angular/core';
import type { DeviceCode } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzConfirmDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';

import {
  GITHUB,
  REMOVE_FAILED,
  accountAbilities,
  accountDetail,
  accountName,
  connectProblem,
} from '../../lib/credentials';
import { CredentialsService } from '../../services/credentials.service';

const COPY = {
  heading: 'Connected accounts',
  lead: 'glave signs git in with these when a command needs it. Their tokens stay locked away on this computer, and are never shown.',
  none: 'No accounts connected yet.',
  can: 'With it, glave can:',
  github: 'GitHub',
  signIn: 'Connect GitHub',
  starting: 'Starting…',
  enter: 'Enter this code on GitHub, then approve glave there:',
  open: 'Open GitHub',
  waiting: 'Waiting for you to approve it on GitHub…',
  cancel: 'Cancel',
  repositories: 'Choose repositories',
  githubToken: 'GitHub token',
  githubTokenHint: 'A fine-grained personal access token, from your GitHub settings.',
  other: 'Another git host',
  address: 'Address',
  addressHint: 'Such as https://gitlab.com, or your own server’s address.',
  token: 'Token',
  tokenHint: 'An access token from that host. It stays on this computer, locked away.',
  connect: 'Connect',
  checking: 'Checking the token…',
  missingToken: 'Paste the token first.',
  missingAddress: 'Give the host’s address first.',
  remove: 'Remove',
  removeHeading: 'Remove this account?',
  removeText: 'glave forgets its token, and git no longer signs in to it from here.',
} as const;

/** What a form is doing. */
type Busy = 'github' | 'host' | null;

/**
 * Settings' connected accounts: the git hosts git signs in to, GitHub by signing in in the
 * browser when the server offers it (else with a token), and any other host with its address
 * and a token. Removing one asks first.
 */
@Component({
  selector: 'app-accounts',
  imports: [
    GlzButtonComponent,
    GlzCardComponent,
    GlzConfirmDialogComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
  ],
  templateUrl: './accounts.component.html',
  styleUrl: './accounts.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AccountsComponent {
  protected readonly copy = COPY;
  private readonly credentials = inject(CredentialsService);
  /** The accounts as the list shows them. */
  protected readonly accounts = computed(() =>
    this.credentials.accounts().map((account) => ({
      id: account.id,
      name: accountName(account),
      detail: accountDetail(account),
      ...accountAbilities(account),
    })),
  );
  protected readonly device = this.credentials.device;
  protected readonly install = this.credentials.install;
  /** The code a sign-in in the browser waits on, while it does. */
  protected readonly code = signal<DeviceCode | null>(null);
  protected readonly githubToken = signal('');
  protected readonly address = signal('');
  protected readonly token = signal('');
  private readonly busy = signal<Busy>(null);
  protected readonly working = computed(() => this.busy() !== null);
  protected readonly signInLabel = computed(() =>
    this.busy() === 'github' ? COPY.starting : COPY.signIn,
  );
  protected readonly githubLabel = computed(() =>
    this.busy() === 'github' ? COPY.checking : COPY.connect,
  );
  protected readonly hostLabel = computed(() =>
    this.busy() === 'host' ? COPY.checking : COPY.connect,
  );
  protected readonly githubProblem = signal<string | null>(null);
  protected readonly hostProblem = signal<string | null>(null);
  protected readonly listProblem = signal<string | null>(null);
  protected readonly confirming = signal(false);
  private removing: string | null = null;
  /** Counts sign-ins, so a cancelled or left one stops waiting. */
  private attempt = 0;
  private gone = false;

  constructor() {
    void this.credentials.load();
    inject(DestroyRef).onDestroy(() => {
      this.gone = true;
    });
  }

  protected typedGithubToken(event: Event): void {
    this.githubToken.set(valueOf(event));
  }

  protected typedAddress(event: Event): void {
    this.address.set(valueOf(event));
  }

  protected typedToken(event: Event): void {
    this.token.set(valueOf(event));
  }

  /** Signs in to GitHub in the browser: shows the code, then waits for the approval. */
  protected async signIn(): Promise<void> {
    if (this.working()) {
      return;
    }
    this.attempt += 1;
    const attempt = this.attempt;
    this.githubProblem.set(null);
    this.busy.set('github');
    try {
      const code = await this.credentials.start();
      this.busy.set(null);
      this.code.set(code);
      await this.credentials.finish(code.flow, () => this.gone || attempt !== this.attempt);
    } catch (error: unknown) {
      if (attempt === this.attempt) {
        this.githubProblem.set(connectProblem(error));
      }
    } finally {
      if (attempt === this.attempt) {
        this.busy.set(null);
        this.code.set(null);
      }
    }
  }

  protected cancel(): void {
    this.attempt += 1;
    this.code.set(null);
  }

  /** Connects GitHub with a pasted token. */
  protected async connectGithub(event: Event): Promise<void> {
    event.preventDefault();
    const token = this.githubToken().trim();
    if (token === '') {
      this.githubProblem.set(COPY.missingToken);
      return;
    }
    if (await this.connected('github', GITHUB, token, this.githubProblem)) {
      this.githubToken.set('');
    }
  }

  /** Connects another host with its address and a token. */
  protected async connectHost(event: Event): Promise<void> {
    event.preventDefault();
    const address = this.address().trim();
    const token = this.token().trim();
    if (address === '') {
      this.hostProblem.set(COPY.missingAddress);
      return;
    }
    if (token === '') {
      this.hostProblem.set(COPY.missingToken);
      return;
    }
    if (await this.connected('host', address, token, this.hostProblem)) {
      this.address.set('');
      this.token.set('');
    }
  }

  /** Connects `url` with `token`, saying what went wrong in `problem`; whether it worked. */
  private async connected(
    busy: Busy,
    url: string,
    token: string,
    problem: WritableSignal<string | null>,
  ): Promise<boolean> {
    if (this.working()) {
      return false;
    }
    problem.set(null);
    this.busy.set(busy);
    try {
      await this.credentials.connect(url, token);
      return true;
    } catch (error: unknown) {
      problem.set(connectProblem(error));
      return false;
    } finally {
      this.busy.set(null);
    }
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
    this.listProblem.set(null);
    try {
      await this.credentials.remove(id);
    } catch {
      this.listProblem.set(REMOVE_FAILED);
    }
  }
}

/** What an input event's field holds now. */
function valueOf(event: Event): string {
  return event.target instanceof HTMLInputElement ? event.target.value : '';
}
