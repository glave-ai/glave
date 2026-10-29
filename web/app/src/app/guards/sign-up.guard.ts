import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { SessionService } from '../services/session.service';

/** Sends a tab opened with a sign-up's emailed link to the page that makes its passkey. */
export const signUpGuard: CanActivateFn = () =>
  inject(SessionService).signupCode() === null || inject(Router).parseUrl('/sign-up');
