/** The language the app is written in: its messages need nothing loaded. */
export const ENGLISH = 'en';

/**
 * The languages the app has messages in besides English, each in `locale/messages.<tag>.json`.
 * For now only the pseudo-locales `npm run i18n` makes from English, to prove every message
 * goes through a translation: `en-XA`, accented and longer, and `ar-XB`, right to left. Only a
 * browser set to one asks for it. A real language joins once a speaker has read it.
 */
export const TRANSLATED = ['en-XA', 'ar-XB'] as const;

/** A language the app has messages in besides English. */
export type Translated = (typeof TRANSLATED)[number];

/** What the app is read in: its messages, and the locale it writes dates and numbers for. */
export interface Language {
  /** English, or the translation whose messages are loaded before the app starts. */
  readonly messages: typeof ENGLISH | Translated;
  /** The browser's tag that chose them (`en-GB`), which dates, times and numbers follow. */
  readonly locale: string;
}

const LANGUAGES: readonly Language['messages'][] = [ENGLISH, ...TRANSLATED];

/**
 * The language for a browser that prefers `preferred` (`navigator.languages`), in its order:
 * the first one the app has, by its whole tag or, for a language with no region of its own
 * (`en`), by its language alone (`en-GB` reads English). English when the app has none of them.
 */
export function chooseLanguage(preferred: readonly string[]): Language {
  for (const tag of preferred) {
    const wanted = tag.toLowerCase();
    const messages =
      LANGUAGES.find((language) => language.toLowerCase() === wanted) ??
      LANGUAGES.find((language) => !language.includes('-') && wanted.startsWith(`${language}-`));
    if (messages !== undefined) {
      return { messages, locale: tag };
    }
  }
  return { messages: ENGLISH, locale: ENGLISH };
}
