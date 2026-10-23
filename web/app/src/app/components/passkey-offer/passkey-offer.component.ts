import { DOCUMENT } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { GlzButtonComponent, GlzDialogComponent } from '@glave/glaze';

import { readStored, storageOf, writeStored } from '../../lib/storage';
import { PLATFORM_AUTHENTICATOR, PasskeysService } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';

/** Where this browser remembers that the person said not now. */
const DECLINED_KEY = 'glave-passkey-offer';

/**
 * Offers, once, to make a passkey for glave on this computer, in the words of its own
 * authenticator ("Sign in with Touch ID next time?"): after a browser here is signed in by
 * glave's link, where no passkey exists yet and the computer has an authenticator of its own.
 * With one, any browser here signs in at the touch of a finger. Not now is remembered in this
 * browser, and asked again nowhere.
 */
@Component({
  selector: 'app-passkey-offer',
  imports: [GlzButtonComponent, GlzDialogComponent],
  templateUrl: './passkey-offer.component.html',
  styleUrl: './passkey-offer.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PasskeyOfferComponent {
  private readonly session = inject(SessionService);
  private readonly passkeys = inject(PasskeysService);
  private readonly storage = storageOf(inject(DOCUMENT));
  private readonly platformAvailable = inject(PLATFORM_AUTHENTICATOR);

  protected readonly copy = {
    text: $localize`:@@passkeyOffer.text:glave makes a passkey for this computer, and from then on any browser here signs in with it in one step. You can remove it in Settings.`,
    later: $localize`:@@passkeyOffer.later:Not now`,
    waiting: $localize`:@@passkeyOffer.waiting:Waiting for your device…`,
  } as const;
  protected readonly open = signal(false);
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected readonly heading = computed(() => {
    switch (this.passkeys.platform) {
      case 'touch-id':
        return $localize`:@@passkeyOffer.headingTouchId:Sign in with Touch ID next time?`;
      case 'windows-hello':
        return $localize`:@@passkeyOffer.headingWindowsHello:Sign in with Windows Hello next time?`;
      default:
        return $localize`:@@passkeyOffer.heading:Sign in with a passkey next time?`;
    }
  });
  protected readonly action = computed(() => {
    if (this.busy()) {
      return this.copy.waiting;
    }
    switch (this.passkeys.platform) {
      case 'touch-id':
        return $localize`:@@passkeyOffer.useTouchId:Use Touch ID`;
      case 'windows-hello':
        return $localize`:@@passkeyOffer.useWindowsHello:Use Windows Hello`;
      default:
        return $localize`:@@passkeyOffer.make:Make a passkey`;
    }
  });

  constructor() {
    void this.consider();
  }

  /** Makes the passkey; says why when it could not, and offers again. */
  protected async make(): Promise<void> {
    this.busy.set(true);
    this.problem.set(null);
    const refused = await this.passkeys.add();
    this.busy.set(false);
    if (refused === null) {
      this.open.set(false);
    } else {
      this.problem.set(refused);
    }
  }

  /** Not now: closed, by its button, Escape or its close button, and not asked again here. */
  protected later(): void {
    writeStored(this.storage, DECLINED_KEY, 'declined');
    this.open.set(false);
  }

  protected openChanged(open: boolean): void {
    if (!open && !this.busy()) {
      this.later();
    }
  }

  /** Opens the offer when glave makes passkeys here, none exists, and none was declined. */
  private async consider(): Promise<void> {
    const offered =
      this.session.local() &&
      this.session.passkeysHere() &&
      !this.session.passkeyMade() &&
      readStored(this.storage, DECLINED_KEY) === null;
    if (offered && (await this.platformAvailable())) {
      this.open.set(true);
    }
  }
}
