import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { SessionService } from '../services/session.service';

/** Sends a tab opened with an invite's link to the page that opens it, signed in or not. */
export const inviteGuard: CanActivateFn = () =>
  inject(SessionService).inviteCode() === null || inject(Router).parseUrl('/invite');
