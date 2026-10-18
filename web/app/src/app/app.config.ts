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
    // Sign the tab in before the router reads the address, which the link's code leaves.
    provideAppInitializer(() => inject(SessionService).start()),
  ],
};
