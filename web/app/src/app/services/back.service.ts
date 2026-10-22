import { DOCUMENT, Location } from '@angular/common';
import { InjectionToken, Service, inject } from '@angular/core';
import { Router } from '@angular/router';

/**
 * The tab's history as the browser's Navigation API keeps it, where the browser has the API:
 * it lists only the pages of glave's own origin that led here, so going back stays in glave.
 */
export const NAVIGATION = new InjectionToken<Pick<Navigation, 'canGoBack'> | null>('NAVIGATION', {
  providedIn: 'root',
  factory: () => inject(DOCUMENT).defaultView?.navigation ?? null,
});

/**
 * Going back, as a page's Back does: to the page before this one when it is glave's, as the
 * browser's own Back would, so the history stays as the person made it; else, when they arrived
 * at the page directly (or the browser keeps no such history), to the page it belongs under.
 */
@Service()
export class BackService {
  private readonly navigation = inject(NAVIGATION);
  private readonly location = inject(Location);
  private readonly router = inject(Router);

  /** Goes back to the page before this one when it is glave's, else to `fallback`. */
  async back(fallback: string): Promise<void> {
    if (this.navigation?.canGoBack === true) {
      this.location.back();
      return;
    }
    await this.router.navigateByUrl(fallback);
  }
}
