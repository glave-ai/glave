/** Which way a language is written, which the whole page follows. */
export type Direction = 'ltr' | 'rtl';

/** The language the app is written in: its messages need nothing loaded. */
export const ENGLISH = 'en';

/**
 * The languages the app has messages in besides English, each in `locale/messages.<tag>.json`,
 * with the way it is written. For now only the pseudo-locales `npm run i18n` makes from
 * English, to prove every message goes through a translation: `en-XA`, accented and longer,
 * and `ar-XB`, right to left. Only a browser set to one asks for it. A real language joins once
 * a speaker has read it.
 */
export const TRANSLATED = { 'en-XA': 'ltr', 'ar-XB': 'rtl' } as const satisfies Readonly<
  Record<string, Direction>
>;

/** A language the app has messages in besides English. */
export type Translated = keyof typeof TRANSLATED;

/** What the app is read in: its messages, and the locale it writes dates and numbers for. */
export interface Language {
  /** English, or the translation whose messages are loaded before the app starts. */
  readonly messages: typeof ENGLISH | Translated;
  /** Which way the messages' language is written. */
  readonly direction: Direction;
  /**
   * The locale dates, times and numbers follow, in the messages' language: the browser's tag
   * that chose them (`en-GB`), or, when the app has none of its languages, English in its
   * first one's region (`en-DE` for `de-DE`).
   */
  readonly locale: string;
}

/** Each language the app has, with the way it is written. */
const LANGUAGES: Readonly<Record<Language['messages'], Direction>> = {
  [ENGLISH]: 'ltr',
  ...TRANSLATED,
};

/** Whether the app has messages in `tag`. */
function isLanguage(tag: string): tag is Language['messages'] {
  return tag in LANGUAGES;
}

const TAGS = Object.keys(LANGUAGES).filter(isLanguage);

/**
 * The language for a browser that prefers `preferred` (`navigator.languages`), in its order:
 * the first one the app has, by its whole tag or, for a language with no region of its own
 * (`en`), by its language alone (`en-GB` reads English); else English. A tag that is no tag is
 * passed over.
 */
export function chooseLanguage(preferred: readonly string[]): Language {
  const tags = preferred.flatMap(canonical);
  for (const tag of tags) {
    const wanted = tag.toLowerCase();
    const messages =
      TAGS.find((language) => language.toLowerCase() === wanted) ??
      TAGS.find((language) => !language.includes('-') && wanted.startsWith(`${language}-`));
    if (messages !== undefined) {
      return { messages, locale: tag, direction: LANGUAGES[messages] };
    }
  }
  const first = tags[0];
  const region = first === undefined ? undefined : new Intl.Locale(first).region;
  return {
    messages: ENGLISH,
    locale: region === undefined ? ENGLISH : `${ENGLISH}-${region}`,
    direction: 'ltr',
  };
}

/** `tag` as BCP 47 writes it (`en-US` for `en-us`), or none when it is not a tag. */
function canonical(tag: string): readonly string[] {
  try {
    return Intl.getCanonicalLocales(tag);
  } catch {
    return [];
  }
}
