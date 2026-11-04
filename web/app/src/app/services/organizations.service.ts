import { Service, computed, effect, inject, signal } from '@angular/core';
import type { OrganizationInfo } from '@glave/client';

import { onChange } from './changes.service';
import { SessionService } from './session.service';

/**
 * The organizations the person belongs to (`organization/list`), their own first, each by the
 * name the server gives it, with the one this tab works in: what the person's menu switches
 * between and the side nav names. Read once signed in, when first asked, again when the server
 * says the organization's brand changed, and forgotten when signed out; a list that cannot be
 * read leaves the menu as it is, with no switching.
 */
@Service()
export class OrganizationsService {
  private readonly session = inject(SessionService);
  private readonly listed = signal<readonly OrganizationInfo[]>([]);
  private pending: Promise<void> | null = null;
  /** How many reads were asked for: only the latest one's answer is kept. */
  private reads = 0;

  /** The person's organizations, their own first, as the server last listed them. */
  readonly organizations = this.listed.asReadonly();
  /** Whether there is anything to switch between: more than one. */
  readonly several = computed(() => this.listed().length > 1);
  /** What the side nav is headed with where the person works in a team, not their own. */
  readonly heading = computed(() => {
    const current = this.listed().find((each) => each.current === true);
    return current === undefined || current.own === true || !this.several() ? null : current.name;
  });

  constructor() {
    onChange('brand', () => {
      this.pending = this.read();
    });
    // Signed out, it forgets them, so whoever signs in next reads their own.
    effect(() => {
      if (!this.session.signedIn()) {
        this.listed.set([]);
        this.pending = null;
        this.reads += 1;
      }
    });
  }

  /** Reads the person's organizations, once; a failed read is tried again next time. */
  load(): Promise<void> {
    if (!this.session.signedIn()) {
      return Promise.resolve();
    }
    this.pending ??= this.read();
    return this.pending;
  }

  /** Works in the organization `organization` from now on. */
  choose(organization: OrganizationInfo): void {
    this.session.chooseOrganization(organization.id);
  }

  private read(): Promise<void> {
    this.reads += 1;
    const read = this.reads;
    return this.session.call('organization/list', {}).then(
      ({ organizations }) => {
        if (read === this.reads) {
          this.listed.set(organizations);
        }
      },
      () => {
        if (read === this.reads) {
          this.pending = null;
        }
      },
    );
  }
}
