import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import type { Ability, CredentialInfo, DeviceCode } from '@glave/client';
import { GlzButtonComponent, GlzConfirmDialogComponent, GlzCopyDirective } from '@glave/glaze';

import { accountDetail, accountName, connectProblem } from '../../lib/credentials';
import { removeFailed } from '../../lib/text';
import { CredentialsService } from '../../services/credentials.service';
import { ServiceRowComponent } from '../service-row/service-row.component';

/** A git host the person connected, as a row shows it. */
interface Host {
  readonly id: string;
  readonly name: string;
  readonly detail: string;
}

/**
 * Connections' git hosts: GitHub as one row, connected by signing in on GitHub (its code to
 * enter, copied as GitHub opens, while the row waits for the approval), then whom it signs in
 * as, where its repositories are chosen, and Disconnect; and each other git host connected by
 * address, with Disconnect. Disconnecting asks first. One sign-in serves git, `gh` and GitHub's
 * tools alike, as the server keeps it.
 */
@Component({
  selector: 'app-github-row',
  imports: [GlzButtonComponent, GlzConfirmDialogComponent, GlzCopyDirective, ServiceRowComponent],
  templateUrl: './github-row.component.html',
  styleUrl: './github-row.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GithubRowComponent {
  protected readonly copy = {
    github: 'GitHub',
    offer: $localize`:@@githubRow.offer:Let glave work in your repositories: clone, push and open pull requests.`,
    expired: $localize`:@@githubRow.expired:Your sign-in expired. Connect again to keep working in your repositories.`,
    connect: $localize`:@@githubRow.connect:Connect`,
    starting: $localize`:@@githubRow.starting:Starting…`,
    enter: $localize`:@@githubRow.enter:Enter this code on GitHub`,
    open: $localize`:@@githubRow.open:Copy code and open GitHub`,
    cancel: $localize`:@@githubRow.cancel:Cancel`,
    waiting: $localize`:@@githubRow.waiting:Waiting while you approve glave on GitHub.`,
    repositories: $localize`:@@githubRow.repositories:Choose repositories`,
    disconnect: $localize`:@@githubRow.disconnect:Disconnect`,
    disconnectText: $localize`:@@githubRow.disconnectText:glave forgets its sign-in, and git and its tools no longer sign in to it from here.`,
  } as const;
  private readonly credentials = inject(CredentialsService);
  /** GitHub, once connected. */
  protected readonly github = computed(
    () =>
      this.credentials.accounts().find((account) => account.url === this.credentials.github()) ??
      null,
  );
  /** What the GitHub row says once connected. */
  protected readonly connected = computed(() => {
    const github = this.github();
    return github === null ? '' : connectedLine(github);
  });
  /** The other git hosts, each as a row. */
  protected readonly hosts = computed<readonly Host[]>(() =>
    this.credentials
      .accounts()
      .filter((account) => account.url !== this.credentials.github())
      .map((account) => ({
        id: account.id,
        name: accountName(account),
        detail: accountDetail(account),
      })),
  );
  protected readonly install = this.credentials.install;
  /** The code a sign-in on GitHub waits on, while it does. */
  protected readonly code = signal<DeviceCode | null>(null);
  protected readonly starting = signal(false);
  /** What the row shows: GitHub's code while a sign-in waits on it, whom it is connected as, that its sign-in expired, or the offer to connect. */
  protected readonly state = computed<'signing' | 'connected' | 'expired' | 'offer'>(() => {
    if (this.code() !== null) {
      return 'signing';
    }
    const github = this.github();
    if (github === null) {
      return 'offer';
    }
    return github.expired === true ? 'expired' : 'connected';
  });
  protected readonly connectLabel = computed(() =>
    this.starting() ? this.copy.starting : this.copy.connect,
  );
  protected readonly problem = signal<string | null>(null);
  protected readonly confirming = signal(false);
  protected readonly disconnectHeading = signal('');
  private removing: string | null = null;
  /** Counts sign-ins, so one cancelled or left stops waiting. */
  private attempt = 0;
  private gone = false;

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      this.gone = true;
    });
  }

  /** Signs in on GitHub: shows the code, then waits for the approval. */
  protected async signIn(): Promise<void> {
    if (this.starting() || this.code() !== null) {
      return;
    }
    this.attempt += 1;
    const attempt = this.attempt;
    this.problem.set(null);
    this.starting.set(true);
    try {
      const code = await this.credentials.start();
      this.starting.set(false);
      this.code.set(code);
      await this.credentials.finish(code.flow, () => this.gone || attempt !== this.attempt);
    } catch (error: unknown) {
      if (attempt === this.attempt) {
        this.problem.set(connectProblem(error));
      }
    } finally {
      if (attempt === this.attempt) {
        this.starting.set(false);
        this.code.set(null);
      }
    }
  }

  protected cancel(): void {
    this.attempt += 1;
    this.code.set(null);
  }

  /** Asks before disconnecting the host `id`, which the person calls `name`. */
  protected ask(id: string, name: string): void {
    this.removing = id;
    this.disconnectHeading.set($localize`:@@githubRow.disconnectHeading:Disconnect ${name}:NAME:?`);
    this.confirming.set(true);
  }

  protected askGithub(): void {
    const github = this.github();
    if (github !== null) {
      this.ask(github.id, this.copy.github);
    }
  }

  protected async decided(yes: boolean): Promise<void> {
    const id = this.removing;
    this.removing = null;
    if (!yes || id === null) {
      return;
    }
    this.problem.set(null);
    try {
      await this.credentials.remove(id);
    } catch {
      this.problem.set(removeFailed());
    }
  }
}

/** What glave does in the repositories GitHub's app was given, all of it as the server says. */
const WORKS: readonly Ability[] = ['read', 'push', 'pull_requests'];

/**
 * What GitHub's row says once connected: whom it signs in as, then what glave can do there,
 * only as far as the server says (`can`): all of it, or, given no repositories yet, to choose
 * some. Where it says nothing, as of a token, the row claims nothing.
 */
function connectedLine(github: CredentialInfo): string {
  const login = github.login ?? '';
  const who =
    login === ''
      ? $localize`:@@githubRow.connectedNoLogin:Connected.`
      : $localize`:@@githubRow.connected:Connected as ${login}:LOGIN:.`;
  const can = github.can ?? null;
  if (can === null) {
    return who;
  }
  if (can.length === 0) {
    return `${who} ${$localize`:@@githubRow.choose:Now choose which repositories glave may use.`}`;
  }
  return WORKS.every((ability) => can.includes(ability))
    ? `${who} ${$localize`:@@githubRow.works:glave can clone, push and open pull requests in the repositories you chose.`}`
    : who;
}
