import type { EnvironmentProviders, Provider } from '@angular/core';
import { bootstrapApplication } from '@angular/platform-browser';

import { appConfig } from './app/app.config';
import { AppComponent } from './app/components/app/app.component';
import { ENGLISH, chooseLanguage } from './app/lib/language';

const { messages, locale, direction } = chooseLanguage(navigator.languages);

/**
 * The app's language, before anything reads a message: a translation's messages loaded (with
 * the providers that put Glaze's own words in it), and the locale Angular's `LOCALE_ID` and
 * the page say, with the way the page is written (its `dir`, which Glaze's layouts and the
 * CDK's `Directionality` follow). English needs nothing loaded; a translation that cannot
 * load leaves English.
 */
async function language(): Promise<readonly (Provider | EnvironmentProviders)[]> {
  try {
    const translated =
      messages === ENGLISH ? [] : await (await import('./translate')).translate(messages);
    $localize.locale = locale;
    document.documentElement.lang = locale;
    document.documentElement.dir = direction;
    return translated;
  } catch (error: unknown) {
    // The app starts in English all the same; the browser console is where this shows.
    // eslint-disable-next-line no-console -- see above
    console.warn(error);
    return [];
  }
}

language()
  .then((providers) =>
    bootstrapApplication(AppComponent, {
      ...appConfig,
      providers: [...appConfig.providers, ...providers],
    }),
  )
  .catch((error: unknown) => {
    // Nothing else can report a failure to start; the browser console is where it shows.
    // eslint-disable-next-line no-console -- see above
    console.error(error);
  });
