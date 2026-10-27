import {
  type ApplicationConfig,
  inject,
  provideAppInitializer,
  provideBrowserGlobalErrorListeners,
} from '@angular/core';
import {
  provideRouter,
  withComponentInputBinding,
  withNavigationErrorHandler,
} from '@angular/router';

import { routes, unloadedPage } from './app.routes';
import { SessionService } from './services/session.service';

/** The app's providers. Change detection is zoneless (Angular's default): signals drive it. */
export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    // A page that cannot open says so in its place (`unloadedPage`).
    provideRouter(routes, withComponentInputBinding(), withNavigationErrorHandler(unloadedPage)),
    // Sign the tab in before the router reads the address, which the link's code leaves.
    provideAppInitializer(() => inject(SessionService).start()),
  ],
};
