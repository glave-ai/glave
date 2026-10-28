import {
  type ApplicationConfig,
  inject,
  provideAppInitializer,
  provideBrowserGlobalErrorListeners,
} from '@angular/core';
import {
  PreloadAllModules,
  provideRouter,
  withComponentInputBinding,
  withNavigationErrorHandler,
  withPreloading,
  withViewTransitions,
} from '@angular/router';

import { crossfadeThreads, routes, unloadedPage } from './app.routes';
import { SessionService } from './services/session.service';
import { ThemeService } from './services/theme.service';

/** The app's providers. Change detection is zoneless (Angular's default): signals drive it. */
export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    // A page that cannot open says so in its place (`unloadedPage`); switching threads
    // crossfades the pane, where the browser can (`crossfadeThreads`). Every page's code is
    // fetched once the first page has opened, so opening another waits on no round trip to the
    // server, however far away it is.
    provideRouter(
      routes,
      withComponentInputBinding(),
      withPreloading(PreloadAllModules),
      withNavigationErrorHandler(unloadedPage),
      withViewTransitions({
        skipInitialTransition: true,
        onViewTransitionCreated: crossfadeThreads,
      }),
    ),
    // Sign the tab in before the router reads the address, which the link's code leaves.
    provideAppInitializer(() => inject(SessionService).start()),
    // The person's theme, before anything is drawn.
    provideAppInitializer(() => {
      inject(ThemeService);
    }),
  ],
};
