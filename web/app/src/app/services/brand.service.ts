import { DOCUMENT } from '@angular/common';
import { Service, effect, inject, signal, untracked } from '@angular/core';
import type { Accent, Logo, OrganizationLook, Shades } from '@glave/client';

import { AppearanceService } from './appearance.service';
import { SessionService } from './session.service';

/** `logo`'s bytes as a Blob, for a URL the page's rules allow (`img-src 'self' blob:`). */
function blobOf({ data, media_type: type }: Logo): Blob {
  const bytes = Uint8Array.from(atob(data), (char) => char.charCodeAt(0));
  return new Blob([bytes], { type });
}

/**
 * The person's organization's logo, for the corner of the side nav, and an admin's
 * reading and keeping of them with its accent (`brand/get`, `brand/set`). The logo is read only
 * when the server says it is another one.
 */
@Service()
export class BrandService {
  private readonly session = inject(SessionService);
  private readonly appearance = inject(AppearanceService);
  private readonly document = inject(DOCUMENT);
  private readonly url = signal<string | null>(null);
  /** Which logo is shown, or being read, so each is read once. */
  private shown: string | null = null;
  /** How many were asked for: only the latest one's answer is shown. */
  private asked = 0;

  /** Its logo, as a URL for this page. */
  readonly logoUrl = this.url.asReadonly();

  constructor() {
    effect(() => {
      const logo = this.appearance.organization()?.logo ?? null;
      untracked(() => void this.show(logo));
    });
  }

  read(): Promise<OrganizationLook> {
    return this.session.call('brand/get', {}).then(({ brand }) => brand);
  }

  /** Keeps the organization's brand, and draws what it now says. */
  async keep(
    name: string,
    logo: string | null,
    accent: Accent,
    locked: boolean,
  ): Promise<OrganizationLook> {
    const { brand } = await this.session.call('brand/set', {
      ...(name.trim() === '' ? {} : { name }),
      ...(logo === null ? {} : { logo }),
      accent,
      locked,
    });
    await this.appearance.read();
    return brand;
  }

  /** Keeps `data`, in base64, for {@link keep} to name, by its hash. */
  async upload(data: string): Promise<string> {
    const { sha256 } = await this.session.call('brand/logo/upload', { data });
    return sha256;
  }

  /** What `color` would be drawn as. */
  preview(color: string, adjust: boolean): Promise<Shades> {
    return this.session.call('appearance/preview', { color, adjust }).then(({ shades }) => shades);
  }

  private async show(logo: string | null): Promise<void> {
    if (logo === this.shown) {
      return;
    }
    this.shown = logo;
    this.asked += 1;
    const asked = this.asked;
    let url: string | null = null;
    if (logo !== null) {
      try {
        const answer = await this.session.call('brand/logo/get', {});
        if (answer.logo && asked === this.asked) {
          url = this.window()?.URL.createObjectURL(blobOf(answer.logo)) ?? null;
        }
      } catch {
        // The name stands in its place until the next change or load, which reads it again.
        if (asked === this.asked) {
          this.shown = null;
        }
      }
    }
    if (asked === this.asked) {
      const before = this.url();
      if (before !== null) {
        this.window()?.URL.revokeObjectURL(before);
      }
      this.url.set(url);
    }
  }

  private window(): (Window & typeof globalThis) | null {
    return this.document.defaultView;
  }
}
