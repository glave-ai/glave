import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { SessionService } from '../services/session.service';

/** Lets a signed-in person in; anyone else goes to the page that says how to sign in. */
export const signedInGuard: CanActivateFn = () =>
  inject(SessionService).signedIn() || inject(Router).parseUrl('/sign-in');
