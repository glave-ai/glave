import type { Provider } from '@angular/core';
import { loadTranslations } from '@angular/localize';
import { GLZ_TEXT } from '@glave/glaze';

import type { Translated } from './app/lib/language';
import { glazeText } from './app/lib/glaze-text';

/** A translation as `npm run i18n` writes it: each message's text by its id. */
interface Catalog {
  readonly translations: Readonly<Record<string, string>>;
}

/** Each translation's messages, loaded only when the app is read in it. */
const CATALOGS: Readonly<Record<Translated, () => Promise<{ readonly default: Catalog }>>> = {
  'en-XA': () => import('./locale/messages.en-XA.json'),
  'ar-XB': () => import('./locale/messages.ar-XB.json'),
};

/**
 * Loads the messages of `language` for `$localize`, before the app reads any, and gives the
 * providers that put Glaze's own words in it. Kept out of the first load: English, the
 * language the app is written in, needs none of this, and Glaze writes English itself.
 */
export async function translate(language: Translated): Promise<readonly Provider[]> {
  const { default: catalog } = await CATALOGS[language]();
  loadTranslations(catalog.translations);
  return [{ provide: GLZ_TEXT, useFactory: glazeText }];
}
