import {
  type ApplicationConfig,
  inject,
  provideAppInitializer,
  provideBrowserGlobalErrorListeners,
} from '@angular/core';
import { provideRouter, withComponentInputBinding } from '@angular/router';

import { routes } from './app.routes';
import { SessionService } from './services/session.service';

/** The app's providers. Change detection is zoneless (Angular's default): signals drive it. */
export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideRouter(routes, withComponentInputBinding()),
    // Take the sign-in token out of the address bar before the router reads it.
    provideAppInitializer(() => {
      inject(SessionService);
    }),
  ],
};
