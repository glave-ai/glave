// The app's messages in each language it has (app/src/locale), in the JSON Angular writes:
// English, extracted from the source's `$localize` messages (`ng extract-i18n`), and every
// translation beside it. The pseudo-locales are made here from English, never by hand:
// `en-XA` accented and longer, as Android and Chrome make it, to show text that skips the
// messages and room a longer language needs; `ar-XB` written right to left.
//
//   node tools/i18n.mjs         extract English and make the pseudo-locales  (npm run i18n)
//   node tools/i18n.mjs check   fail when a translation lacks one of English's messages, has
//                               one English does not, or differs from it in placeholders or
//                               plurals, or a pseudo-locale is not what this makes  (prebuild)
//   node tools/i18n.mjs fresh   check, and fail when English is not what the source says
//                               (npm run i18n:check, in npm run check)
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const web = new URL('..', import.meta.url).pathname;
const LOCALE = join(web, 'app/src/locale');
const SOURCE = 'messages.json';
const PSEUDO = { 'en-XA': accentedLonger, 'ar-XB': rightToLeft };

/** A placeholder, as Angular writes one: `{$COUNT}`. */
const PLACEHOLDER = /\{\$[^}]+\}/gu;
/** The cases a plural may have: CLDR's categories, and exact numbers. */
const CASE = /^(=\d+|zero|one|two|few|many|other)$/u;
/** Each letter and its accented look-alike, for `en-XA`. */
const PLAIN = [...'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'];
const ACCENTED = [...'åƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ'];
const ACCENTS = new Map(PLAIN.map((char, index) => [char, ACCENTED[index]]));
/** What makes `en-XA` longer, a word at a time. */
const NUMBERS = ['one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine', 'ten'];

const command = process.argv[2] ?? 'extract';
const problems = [];
try {
  run(command);
} catch (error) {
  problems.push(error instanceof Error ? error.message : String(error));
}
if (problems.length > 0) {
  console.error(`The app's messages need work:\n  ${problems.join('\n  ')}`);
  process.exit(1);
}

/** Does what `command` says, adding to `problems` what is wrong. */
function run(command) {
  if (command === 'extract') {
    writeCatalog(SOURCE, 'en', extract());
  } else if (command === 'fresh') {
    freshness();
  } else if (command !== 'check') {
    problems.push(`no command "${command}": extract, check or fresh`);
    return;
  }
  const english = readCatalog(SOURCE);
  // English's own plurals first: a pseudo-locale is made from them.
  for (const [id, message] of Object.entries(english)) {
    problems.push(...pluralProblems(message).map((problem) => `en ${id}: ${problem}`));
  }
  if (problems.length > 0) {
    return;
  }
  if (command === 'extract') {
    for (const [tag, make] of Object.entries(PSEUDO)) {
      writeCatalog(`messages.${tag}.json`, tag, pseudo(english, make));
    }
  } else {
    check(english);
  }
}

/** English as the source says it: Angular extracts it, by id, ordered by id. */
function extract() {
  const out = mkdtempSync(join(tmpdir(), 'glave-i18n-'));
  try {
    const ng = join(web, 'node_modules/.bin/ng');
    const args = ['extract-i18n', 'app', '--format', 'json', '--output-path', out];
    execFileSync(ng, [...args, '--out-file', SOURCE, '--progress', 'false'], {
      cwd: web,
      stdio: ['ignore', 'ignore', 'inherit'],
    });
    return JSON.parse(readFileSync(join(out, SOURCE), 'utf8')).translations;
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
}

/** Fails when the English kept here is not what the source's messages say now. */
function freshness() {
  const kept = JSON.stringify(sorted(readCatalog(SOURCE)));
  if (JSON.stringify(sorted(extract())) !== kept) {
    problems.push('the source changed its messages: run `npm run i18n` and commit app/src/locale');
  }
}

/** Every translation against English, and each pseudo-locale against what this makes. */
function check(source) {
  const files = readdirSync(LOCALE).filter((file) => /^messages\..+\.json$/u.test(file));
  for (const file of files) {
    const tag = file.slice('messages.'.length, -'.json'.length);
    const translated = readCatalog(file);
    const locale = JSON.parse(readFileSync(join(LOCALE, file), 'utf8')).locale;
    if (locale !== tag) {
      problems.push(`${file} says its language is ${String(locale)}, not ${tag}`);
    }
    for (const id of Object.keys(source)) {
      if (!(id in translated)) {
        problems.push(`${tag} lacks ${id}`);
      } else if (shape(translated[id]) !== shape(source[id])) {
        problems.push(
          `${tag} ${id}: placeholders ${shape(translated[id])}, English has ${shape(source[id])}`,
        );
      } else {
        problems.push(
          ...pluralProblems(translated[id]).map((problem) => `${tag} ${id}: ${problem}`),
        );
      }
    }
    for (const id of Object.keys(translated)) {
      if (!(id in source)) {
        problems.push(`${tag} has ${id}, which English does not`);
      }
    }
    const make = PSEUDO[tag];
    if (make !== undefined && JSON.stringify(translated) !== JSON.stringify(pseudo(source, make))) {
      problems.push(`${tag} is not what npm run i18n makes from English: run it again`);
    }
  }
  for (const tag of Object.keys(PSEUDO)) {
    if (!files.includes(`messages.${tag}.json`)) {
      problems.push(`${tag} is missing: run npm run i18n`);
    }
  }
}

/** A message's placeholders, each once and in order of name: what a translation must keep. */
function shape(message) {
  return JSON.stringify([...new Set(message.match(PLACEHOLDER) ?? [])].sort());
}

/**
 * What is wrong with the plurals in `message`: one that does not close, a case that is no case,
 * none for `other`, or a case holding braces (a placeholder or another plural), which the app's
 * plurals (`lib/plural.ts`) do not write: a placeholder's value is put in before the plural is
 * read.
 */
function pluralProblems(message) {
  const found = [];
  try {
    walk(message, {
      text: () => '',
      plural: (cases) => {
        for (const [name, body] of cases) {
          if (!CASE.test(name)) {
            found.push(`"${name}" is not a plural case`);
          }
          if (/[{}]/u.test(body)) {
            found.push(
              `the "${name}" case holds braces, a placeholder or a plural: put it outside`,
            );
          }
        }
        if (!cases.some(([name]) => name === 'other')) {
          found.push('a plural has no "other" case');
        }
      },
    });
  } catch (error) {
    found.push(error instanceof Error ? error.message : String(error));
  }
  return found;
}

/** `catalog` with `make` applied to the words of each message, leaving placeholders whole. */
function pseudo(catalog, make) {
  return Object.fromEntries(Object.entries(catalog).map(([id, message]) => [id, make(message)]));
}

/** `[Šéţţîñĝš one two]`: every letter accented, a third longer, bracketed to show a cut. */
function accentedLonger(message) {
  let letters = 0;
  const accented = walk(message, {
    text: (text) => {
      letters += text.replace(/\s/gu, '').length;
      return [...text].map((char) => ACCENTS.get(char) ?? char).join('');
    },
  });
  const padding = [];
  for (let added = 0; padding.length === 0 || added < letters * 0.3;) {
    const word = NUMBERS[padding.length % NUMBERS.length];
    padding.push(word);
    added += word.length + 1;
  }
  return `[${accented} ${padding.join(' ')}]`;
}

/** Each word between the marks that write it right to left, as Android's `ar-XB` does. */
function rightToLeft(message) {
  return walk(message, {
    text: (text) => text.replace(/\S+/gu, (word) => `\u200f\u202e${word}\u202c\u200f`),
  });
}

/**
 * `message` rebuilt with its words given to `text`, and each ICU plural's cases to `plural`
 * (as `[name, words]` pairs): placeholders and the plural's own syntax stay as they are, and so
 * does `#`, the number, in a plural's case (`inCase`).
 */
function walk(message, { text, plural = () => {} }, inCase = false) {
  let out = '';
  let words = '';
  const flush = () => {
    out += text(words);
    words = '';
  };
  for (let index = 0; index < message.length;) {
    const placeholder = /^\{\$[^}]+\}/u.exec(message.slice(index));
    const head = /^\{(\{\$[^}]+\}|-?\d+)\s*,\s*plural\s*,/u.exec(message.slice(index));
    if (placeholder !== null) {
      flush();
      out += placeholder[0];
      index += placeholder[0].length;
    } else if (head !== null) {
      flush();
      const parsed = pluralAt(message, index + head[0].length);
      plural(parsed.cases);
      const cases = parsed.cases.map(
        ([name, body]) => `${name} {${walk(body, { text, plural }, true)}}`,
      );
      out += `${head[0]} ${cases.join(' ')}}`;
      index = parsed.end;
    } else if (inCase && message[index] === '#') {
      flush();
      out += '#';
      index += 1;
    } else {
      words += message[index];
      index += 1;
    }
  }
  flush();
  return out;
}

/** The cases of the plural whose cases begin at `at`, and where the plural ends. */
function pluralAt(message, at) {
  const cases = [];
  let index = at;
  for (;;) {
    const rest = message.slice(index);
    const closing = /^\s*\}/u.exec(rest);
    if (closing !== null) {
      return { cases, end: index + closing[0].length };
    }
    const selector = /^\s*([^\s{}]+)\s*\{/u.exec(rest);
    if (selector === null) {
      throw new Error(`a plural that does not close in "${message}"`);
    }
    let depth = 1;
    let end = index + selector[0].length;
    for (; depth > 0 && end < message.length; end += 1) {
      depth += message[end] === '{' ? 1 : message[end] === '}' ? -1 : 0;
    }
    if (depth > 0) {
      throw new Error(`a plural case that does not close in "${message}"`);
    }
    cases.push([selector[1], message.slice(index + selector[0].length, end - 1)]);
    index = end;
  }
}

function readCatalog(file) {
  return JSON.parse(readFileSync(join(LOCALE, file), 'utf8')).translations;
}

/**
 * Writes a catalog, ordered by id so it changes only where messages do, with the marks that
 * turn text's direction escaped (`\u202e`): unseen in an editor, they would hide what a line
 * says.
 */
function writeCatalog(file, locale, translations) {
  const json = JSON.stringify({ locale, translations: sorted(translations) }, null, 2).replace(
    /[\u200e\u200f\u202a-\u202e\u2066-\u2069]/gu,
    (mark) => `\\u${mark.charCodeAt(0).toString(16).padStart(4, '0')}`,
  );
  writeFileSync(join(LOCALE, file), `${json}\n`);
}

function sorted(translations) {
  return Object.fromEntries(Object.entries(translations).sort(([a], [b]) => (a < b ? -1 : 1)));
}
