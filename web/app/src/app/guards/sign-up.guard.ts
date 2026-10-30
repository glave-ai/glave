import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { SessionService } from '../services/session.service';

/**
 * Sends a tab opened with an emailed link, a sign-up's or one to get back in, to the page that
 * makes its passkey.
 */
export const signUpGuard: CanActivateFn = () => {
  const session = inject(SessionService);
  const router = inject(Router);
  if (session.signupCode() !== null) {
    return router.parseUrl('/sign-up');
  }
  return session.recoverCode() === null || router.parseUrl('/recover');
};
