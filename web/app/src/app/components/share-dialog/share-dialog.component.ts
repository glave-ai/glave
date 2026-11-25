import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  model,
  output,
  signal,
} from '@angular/core';
import type { Audience, Methods, ShareInfo, ShareGetParams } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCopyButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { whyNot } from '../../lib/made';
import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';

/** Who may open a file, as the dialog offers it. */
type Choice = 'only_you' | 'people' | 'anyone';

/**
 * Sharing one file at a link of its own (`share/get`, `share/set`): who may open it, Only you
 * or Anyone with the link, as far as the server offers; once shared, the link to copy; and, on
 * a card whose file changed since it was shared, Update to this version, the same link. Only
 * you stops sharing it, and its link stops working. Tells its parent each time the share
 * changed, so the card or the Library says so.
 */
@Component({
  selector: 'app-share-dialog',
  imports: [
    GlzButtonComponent,
    GlzCopyButtonComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
  ],
  templateUrl: './share-dialog.component.html',
  styleUrl: './share-dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShareDialogComponent {
  /** The file, as `share/get` names it. */
  readonly of = input.required<ShareGetParams>();
  /** Its name, as its card shows it. */
  readonly name = input.required<string>();
  /** Whether the dialog is open. */
  readonly open = model(false);
  /** The share as it is now, each time it changed; `null` once it stopped. */
  readonly changed = output<ShareInfo | null>();

  private readonly session = inject(SessionService);
  private readonly server = inject(ServerService);
  /** Whether the server lets the person share with specific people: where it sends email. */
  protected readonly offersPeople = computed(() => this.server.shareChoices().includes('people'));
  /** The addresses typed for Specific people, as the person wrote them. */
  protected readonly emails = signal('');
  protected readonly choosingPeople = computed(() => this.choice() === 'people');
  protected readonly copy = {
    who: $localize`:@@shareDialog.who:Who can open it`,
    onlyYou: $localize`:@@shareDialog.onlyYou:Only you`,
    onlyYouHint: $localize`:@@shareDialog.onlyYouHint:No one else can open it.`,
    stopHint: $localize`:@@shareDialog.stopHint:Stops sharing. The link stops working.`,
    people: $localize`:@@shareDialog.people:Specific people`,
    peopleHint: $localize`:@@shareDialog.peopleHint:Only the people you add. They prove it's them with a code sent to their email; no account needed.`,
    emails: $localize`:@@shareDialog.emails:Their emails, separated by commas`,
    anyone: $localize`:@@shareDialog.anyone:Anyone with the link`,
    anyoneHint: $localize`:@@shareDialog.anyoneHint:Whoever has the link can view it without signing in.`,
    share: $localize`:@@shareDialog.share:Share`,
    save: $localize`:@@shareDialog.save:Save`,
    saving: $localize`:@@shareDialog.saving:Saving…`,
    link: $localize`:@@shareDialog.link:Link`,
    copyLink: $localize`:@@shareDialog.copyLink:Copy link`,
    note: $localize`:@@shareDialog.note:The link can't be guessed and is never listed in search. They see it as it is now; you choose when to update it.`,
    changedSince: $localize`:@@shareDialog.changedSince:This file changed since you shared it.`,
    update: $localize`:@@shareDialog.update:Update to this version`,
    updateHint: $localize`:@@shareDialog.updateHint:Same link; people who open it see the new version.`,
    done: $localize`:@@shareDialog.done:Done`,
  } as const;
  protected readonly heading = computed(
    () => $localize`:@@shareDialog.heading:Share “${this.name()}:NAME:”`,
  );
  /** The share as the server last said; `null` while the file is not shared. */
  protected readonly share = signal<ShareInfo | null>(null);
  protected readonly choice = signal<Choice>('only_you');
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly link = computed(() => this.share()?.link ?? null);
  protected readonly onlyYouHint = computed(() =>
    this.share() === null ? this.copy.onlyYouHint : this.copy.stopHint,
  );
  /** Whether choosing would change nothing: the same as the share now. */
  private readonly unchanged = computed(() => {
    const share = this.share();
    const same = this.choice() === chosen(share);
    return this.choice() === 'people'
      ? same && listed(this.emails()).join() === peopleOf(share).join()
      : same;
  });
  /** Whether Share or Save does nothing now: busy, or nothing to change. */
  protected readonly blocked = computed(() => this.busy() || !this.loaded() || this.unchanged());
  /** Whether the share as it is now was read: until then, and if it can't be, nothing to choose. */
  protected readonly loaded = signal(false);
  protected readonly applyLabel = computed(() => {
    if (this.busy()) {
      return this.copy.saving;
    }
    return this.share() === null ? this.copy.share : this.copy.save;
  });
  /** Whether the share holds an older version of the file than this card's. */
  protected readonly older = computed(() => {
    const share = this.share();
    const of = this.of();
    return share !== null && of.kind === 'file' && share.sha256 !== of.sha256;
  });

  constructor() {
    effect(() => {
      if (this.open()) {
        void this.read(this.of());
      }
    });
  }

  protected chose(value: string | null): void {
    this.choice.set(value === 'anyone' || value === 'people' ? value : 'only_you');
  }

  protected typed(event: Event): void {
    if (event.target instanceof HTMLTextAreaElement) {
      this.emails.set(event.target.value);
    }
  }

  /** Keeps the choice: shares the file, or stops sharing it. */
  protected async apply(): Promise<void> {
    if (this.unchanged()) {
      return;
    }
    const choice = this.choice();
    const audience: Audience | null =
      choice === 'people'
        ? { kind: 'people', people: listed(this.emails()) }
        : choice === 'anyone'
          ? { kind: 'anyone' }
          : null;
    await this.set(audience, false);
  }

  /** Moves the share to this card's version, the same link, for the same people. */
  protected async update(): Promise<void> {
    const audience = this.share()?.audience ?? null;
    await this.set(audience, true);
  }

  protected close(): void {
    this.open.set(false);
  }

  /**
   * Reads the share as it is now, busy meanwhile, so no choice is made on what an answer still
   * coming would replace; one that can't be read says why, and offers nothing to choose.
   */
  private async read(of: ShareGetParams): Promise<void> {
    this.problem.set(null);
    this.loaded.set(false);
    this.busy.set(true);
    try {
      const share = held(await this.session.call('share/get', of));
      this.share.set(share);
      this.choice.set(chosen(share));
      this.emails.set(peopleOf(share).join(', '));
      this.loaded.set(true);
    } catch (error: unknown) {
      this.problem.set(whyNot(error));
    } finally {
      this.busy.set(false);
    }
  }

  private async set(audience: Audience | null, update: boolean): Promise<void> {
    if (this.busy()) {
      return;
    }
    this.busy.set(true);
    this.problem.set(null);
    try {
      const params = audience === null ? { ...this.of() } : { ...this.of(), audience, update };
      const share = held(await this.session.call('share/set', params));
      this.share.set(share);
      this.choice.set(chosen(share));
      this.changed.emit(share);
    } catch (error: unknown) {
      this.problem.set(whyNot(error));
    } finally {
      this.busy.set(false);
    }
  }
}

/** The share an answer holds: none where it says only that it is done. */
function held(answered: Methods['share/set']['result']): ShareInfo | null {
  return 'share' in answered ? answered.share : null;
}

/** The choice `share` is. */
function chosen(share: ShareInfo | null): Choice {
  if (share === null) {
    return 'only_you';
  }
  return share.audience.kind === 'people' ? 'people' : 'anyone';
}

/** The addresses `share` lists, where it is for specific people. */
function peopleOf(share: ShareInfo | null): readonly string[] {
  return share?.audience.kind === 'people' ? share.audience.people : [];
}

/** The addresses `typed` names: split at commas, semicolons and spaces, the empty left out. */
function listed(typed: string): string[] {
  return typed
    .split(/[\s,;]+/u)
    .map((each) => each.trim())
    .filter((each) => each !== '');
}
