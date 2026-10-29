import { Service, inject, signal } from '@angular/core';
import { ApiError, ErrorCode, type ShownInvite } from '@glave/client';

import { unreached } from '../lib/text';
import { PasskeysService } from './passkeys.service';
import { SessionService } from './session.service';

/** What the invite the tab was opened with opens, as the server said. */
export type Opened =
  | { readonly kind: 'reading' }
  | { readonly kind: 'shown'; readonly shown: ShownInvite }
  | { readonly kind: 'gone' }
  | { readonly kind: 'failed'; readonly problem: string };

/**
 * The invite the tab was opened with (`#invite=…`): what it opens, as the server says, to anyone
 * holding it; then joining with it, by making a passkey, or, signed in already, by accepting it;
 * or putting it aside. Each says `null` when it worked, else why not.
 */
@Service()
export class InviteService {
  private readonly session = inject(SessionService);
  private readonly passkeys = inject(PasskeysService);
  private readonly read = signal<Opened>({ kind: 'reading' });

  /** What the invite opens, once the server has said. */
  readonly opened = this.read.asReadonly();
  /** Asks the server what the tab's invite opens. */
  async load(): Promise<void> {
    const code = this.session.inviteCode();
    if (code === null) {
      this.read.set({ kind: 'gone' });
      return;
    }
    try {
      const { shown } = await this.session.call('invite/show', { code });
      this.read.set({ kind: 'shown', shown });
    } catch (error: unknown) {
      // A sign-in that ended, as for someone taken out, is forgotten: asked again signed out.
      if (error instanceof ApiError && error.signedOut && !this.session.signedIn()) {
        await this.load();
        return;
      }
      if (error instanceof ApiError && error.code === ErrorCode.NO_INVITE) {
        this.read.set({ kind: 'gone' });
      } else {
        const problem = error instanceof ApiError ? error.message : unreached();
        this.read.set({ kind: 'failed', problem });
      }
    }
  }

  /** Joins by making a passkey on this device: a new account, signed in. */
  async joinWithPasskey(): Promise<string | null> {
    const code = this.session.inviteCode();
    if (code === null) {
      return null;
    }
    const problem = await this.passkeys.join(code);
    if (problem === null) {
      this.session.clearInvite();
    }
    return problem;
  }

  /** Joins as the person signed in: the invite's organization is theirs too. */
  async accept(): Promise<string | null> {
    const code = this.session.inviteCode();
    if (code === null) {
      return null;
    }
    try {
      await this.session.call('invite/accept', { code });
      this.session.clearInvite();
      return null;
    } catch (error: unknown) {
      return error instanceof ApiError ? error.message : unreached();
    }
  }

  /** Puts the invite aside, for now. */
  later(): void {
    this.session.clearInvite();
  }
}
