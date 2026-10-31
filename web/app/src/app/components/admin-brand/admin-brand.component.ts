import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  type ElementRef,
  Injector,
  afterNextRender,
  computed,
  effect,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { type Accent, ApiError, type Shades } from '@glave/client';
import {
  GlzAvatarComponent,
  GlzButtonComponent,
  GlzCardComponent,
  GlzCheckboxComponent,
  GlzColorChoiceComponent,
  type GlzColorOption,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';

import { AccentShadeDirective } from '../../directives/accent-shade.directive';
import { brandSwatches } from '../../lib/accents';
import { base64Of } from '../../lib/attachments';
import { sentence, unreached } from '../../lib/text';
import { PLAIN } from '../../services/appearance.service';
import { BrandService } from '../../services/brand.service';
import { PageHeaderComponent } from '../page-header/page-header.component';

/**
 * Admin › Brand: how glave looks and what it's called for everyone in the organization. Its name,
 * its logo, its accent with how that reads in light and dark, and the switch that makes everyone
 * use it. Nothing is kept until Save. The values shown are the server's, for the color chosen.
 */
@Component({
  selector: 'app-admin-brand',
  imports: [
    AccentShadeDirective,
    GlzAvatarComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzCheckboxComponent,
    GlzColorChoiceComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    PageHeaderComponent,
  ],
  templateUrl: './admin-brand.component.html',
  styleUrl: './admin-brand.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminBrandComponent {
  protected readonly copy = {
    heading: $localize`:@@adminBrand.heading:Brand`,
    intro: $localize`:@@adminBrand.intro:How glave looks and what it’s called for everyone in your organization.`,
    name: $localize`:@@adminBrand.name:Organization name`,
    logo: $localize`:@@adminBrand.logo:Logo`,
    logoHint: $localize`:@@adminBrand.logoHint:Shown at the top of the sidebar, for everyone in your organization.`,
    upload: $localize`:@@adminBrand.upload:Upload a logo`,
    remove: $localize`:@@adminBrand.remove:Remove`,
    accent: $localize`:@@adminBrand.accent:Accent color`,
    accentHint: $localize`:@@adminBrand.accentHint:Pick one of these, or your exact brand color.`,
    any: $localize`:@@adminBrand.any:Pick your own color`,
    inLight: $localize`:@@adminBrand.inLight:In light`,
    inDark: $localize`:@@adminBrand.inDark:In dark`,
    send: $localize`:@@adminBrand.send:Send`,
    link: $localize`:@@adminBrand.link:A link in an answer`,
    adjust: $localize`:@@adminBrand.adjust:Adjust for readability in light and dark`,
    own: $localize`:@@adminBrand.own:People’s own choices`,
    locked: $localize`:@@adminBrand.locked:Everyone uses this accent`,
    save: $localize`:@@adminBrand.save:Save`,
    saved: $localize`:@@adminBrand.saved:Saved. It shows for everyone the next time they open glave.`,
  } as const;
  protected readonly logoKinds = 'image/png,image/jpeg,image/webp';
  protected readonly accents: readonly GlzColorOption[] = [
    { value: PLAIN, label: $localize`:@@adminBrand.plain:No accent` },
    ...brandSwatches(),
  ];
  private readonly service = inject(BrandService);
  private readonly injector = inject(Injector);
  private readonly picker = viewChild.required<ElementRef<HTMLInputElement>>('picker');
  private readonly pickButton = viewChild.required<ElementRef<HTMLButtonElement>>('pickButton');

  protected readonly name = signal('');
  protected readonly accent = signal<string>(PLAIN);
  protected readonly adjust = signal(true);
  protected readonly locked = signal(false);
  protected readonly shades = signal<Shades | null>(null);
  /** The logo that Save keeps, by its hash. */
  private readonly logo = signal<string | null>(null);
  /** The file picked, as a URL for this page, until it is kept or another is picked. */
  private readonly picked = signal<string | null>(null);
  protected readonly logoUrl = computed(() =>
    this.logo() === null ? null : (this.picked() ?? this.service.logoUrl()),
  );
  protected readonly problem = signal<string | null>(null);
  /** Whether the form holds what the server keeps, so Save can't overwrite what it never read. */
  protected readonly ready = signal(false);
  /** Whether a logo or a save is on its way, so what is saved is what shows. */
  protected readonly busy = signal(false);
  protected readonly saved = signal(false);
  protected readonly savedLine = computed(() => (this.saved() ? this.copy.saved : ''));
  protected readonly unsaveable = computed(() => !this.ready() || this.busy());
  /** How many previews were asked for: only the latest one's answer is shown. */
  private previews = 0;

  protected readonly shownName = computed(() => this.name().trim() || this.copy.logo);
  protected readonly adjustLine = computed(() =>
    this.adjust()
      ? $localize`:@@adminBrand.adjusted:glave tunes your color a little so text stays easy to read.`
      : $localize`:@@adminBrand.exact:Your exact color, everywhere. Some text may be hard to read.`,
  );
  protected readonly lockedLine = computed(() =>
    this.locked()
      ? $localize`:@@adminBrand.lockedOn:Nobody in your organization can change it.`
      : $localize`:@@adminBrand.lockedOff:This is the default. People can change their own in Settings.`,
  );

  constructor() {
    void this.load();
    inject(DestroyRef).onDestroy(() => {
      this.showPicked(null);
    });
    // The server's values for what is chosen now.
    effect(() => {
      void this.preview(this.accent(), this.adjust());
    });
  }

  protected named(event: Event): void {
    this.name.set((event.target as HTMLInputElement).value);
    this.saved.set(false);
  }

  protected pickAccent(value: string | null): void {
    if (value !== null) {
      this.accent.set(value);
      this.saved.set(false);
    }
  }

  protected pickAdjust(adjust: boolean): void {
    this.adjust.set(adjust);
    this.saved.set(false);
  }

  protected pickLogo(): void {
    this.picker().nativeElement.click();
  }

  protected async pickedLogo(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (file === undefined) {
      return;
    }
    this.busy.set(true);
    this.saved.set(false);
    await this.guarded(async () => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      this.logo.set(await this.service.upload(base64Of(bytes)));
      this.showPicked(URL.createObjectURL(file));
    });
    this.busy.set(false);
  }

  protected removeLogo(): void {
    this.logo.set(null);
    this.showPicked(null);
    this.saved.set(false);
    // The button pressed goes: focus goes to the one beside it.
    afterNextRender(
      () => {
        this.pickButton().nativeElement.focus();
      },
      { injector: this.injector },
    );
  }

  protected pickLocked(locked: boolean): void {
    this.locked.set(locked);
    this.saved.set(false);
  }

  protected async save(): Promise<void> {
    this.busy.set(true);
    this.saved.set(false);
    await this.guarded(async () => {
      const value = this.accent();
      const accent: Accent =
        value === PLAIN ? PLAIN : { color: { color: value, adjust: this.adjust() } };
      await this.service.keep(this.name(), this.logo(), accent, this.locked());
      this.saved.set(true);
    });
    this.busy.set(false);
  }

  private showPicked(url: string | null): void {
    const before = this.picked();
    if (before !== null) {
      URL.revokeObjectURL(before);
    }
    this.picked.set(url);
  }

  private async load(): Promise<void> {
    await this.guarded(async () => {
      const brand = await this.service.read();
      this.name.set(brand.name ?? '');
      this.logo.set(brand.logo ?? null);
      this.locked.set(brand.locked);
      if (brand.accent === PLAIN) {
        this.accent.set(PLAIN);
      } else {
        this.accent.set(brand.accent.color.color);
        this.adjust.set(brand.accent.color.adjust);
      }
      this.ready.set(true);
    });
  }

  private async preview(value: string, adjust: boolean): Promise<void> {
    this.previews += 1;
    const asked = this.previews;
    if (value === PLAIN) {
      this.shades.set(null);
      return;
    }
    await this.guarded(async () => {
      const shades = await this.service.preview(value, adjust);
      if (asked === this.previews) {
        this.shades.set(shades);
      }
    }, false);
  }

  /** Runs `work`, saying why when it is refused; done, that goes if `clears` says. */
  private async guarded(work: () => Promise<void>, clears = true): Promise<void> {
    try {
      await work();
      if (clears) {
        this.problem.set(null);
      }
    } catch (error: unknown) {
      this.problem.set(error instanceof ApiError ? sentence(error.message) : unreached());
    }
  }
}
