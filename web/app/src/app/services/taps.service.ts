import { DOCUMENT } from '@angular/common';
import { Service, inject } from '@angular/core';
import { Router } from '@angular/router';

import { tapped } from '../lib/tapped';
import { OrganizationsService } from './organizations.service';
import { SessionService } from './session.service';

/** A message glave's service worker posted this window, and where it waits for the answer. */
type Posted = Pick<MessageEvent, 'data' | 'ports'>;

/**
 * Opens what a tapped notification names in this window, when glave's service worker asks it
 * to (`push-worker.js`): a thread of the organization this tab works in, without loading the
 * page again, saying so to the worker. One of another organization it leaves to the worker,
 * which loads it there. It listens from the moment push's own pieces load (`PushService`),
 * on a thread's page and in Settings, so the app's first page carries none of it; on any other
 * page the worker loads the thread.
 */
@Service()
export class TapsService {
  private readonly router = inject(Router);
  private readonly session = inject(SessionService);
  private readonly organizations = inject(OrganizationsService);

  constructor() {
    const view = inject(DOCUMENT).defaultView;
    if (view !== null && 'serviceWorker' in view.navigator) {
      view.navigator.serviceWorker.addEventListener('message', (posted) => {
        this.heard(posted);
      });
    }
  }

  /** Takes in what the worker posted. */
  heard(posted: Posted): void {
    const asked = tapped(posted.data);
    if (asked === null || !this.worksIn(asked.org)) {
      return;
    }
    posted.ports[0]?.postMessage('opened');
    void this.router.navigateByUrl(asked.page);
  }

  /** Whether this tab works in the organization `org`, as far as it knows. */
  private worksIn(org: string | null): boolean {
    if (org === null) {
      return true;
    }
    const chosen = this.session.organization();
    if (chosen !== null) {
      return chosen === org;
    }
    return this.organizations.organizations().find((each) => each.current === true)?.id === org;
  }
}
