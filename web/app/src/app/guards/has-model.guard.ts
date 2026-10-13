import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { ModelsService } from '../services/models.service';

/** Lets the person in once a model is set up; on a first run, to the page that adds one. */
export const hasModelGuard: CanActivateFn = async () => {
  const router = inject(Router);
  return (await inject(ModelsService).none()) ? router.parseUrl('/connect') : true;
};
