import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { ServerService } from '../services/server.service';

/**
 * Lets the organization's Admins and Owners into Admin, once the server has said who the person
 * is to it; anyone else, and anyone on a server with no organization, goes to their Chats.
 */
export const adminGuard: CanActivateFn = async () => {
  const server = inject(ServerService);
  const router = inject(Router);
  await server.load();
  return server.admin() || router.parseUrl('/');
};
