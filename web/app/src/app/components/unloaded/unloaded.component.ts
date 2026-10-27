import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  InjectionToken,
  booleanAttribute,
  computed,
  inject,
  input,
} from '@angular/core';
import { GlzButtonComponent } from '@glave/glaze';

import { ServerService } from '../../services/server.service';

/** Loads the page again, as the browser's own Reload does. */
export const RELOAD = new InjectionToken<() => void>('RELOAD', {
  providedIn: 'root',
  factory: () => {
    const view = inject(DOCUMENT).defaultView;
    return () => {
      view?.location.reload();
    };
  },
});

/**
 * What stands in the place of a part of the app whose code could not load, a page or a deferred
 * block's: a plain sentence and Reload, which loads the page again. Its words are the app's own,
 * for when glave can't be reached: when the server now says another version than the one the
 * page loaded with, glave was updated while the tab was open and the code the page asked for is
 * gone; else the network dropped, or glave went out of reach. As a `page`, it is the page.
 */
@Component({
  selector: 'app-unloaded',
  imports: [GlzButtonComponent],
  templateUrl: './unloaded.component.html',
  styleUrl: './unloaded.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { 'data-testid': 'unloaded', '[class.page]': 'page()' },
})
export class UnloadedComponent {
  /** Whether it is the whole page, in place of one that could not open. */
  readonly page = input(false, { transform: booleanAttribute });

  protected readonly copy = {
    updated: $localize`:@@unloaded.updated:glave was updated. Reload to continue.`,
    failed: $localize`:@@unloaded.failed:This part of glave could not load.`,
    reload: $localize`:@@unloaded.reload:Reload`,
  } as const;
  private readonly server = inject(ServerService);
  private readonly reloadPage = inject(RELOAD);
  protected readonly sentence = computed(() =>
    this.server.updated() ? this.copy.updated : this.copy.failed,
  );

  constructor() {
    void this.server.recheck();
  }

  protected reload(): void {
    this.reloadPage();
  }
}
