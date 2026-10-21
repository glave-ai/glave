/** Where a plural begins: its number, as `$localize` put it in, then the word `plural`. */
const START = /\{\s*(-?\d+(?:\.\d+)?)\s*,\s*plural\s*,/gu;

/** One case of a plural: `one`, `other`, or an exact `=0`, before its braces. */
const SELECTOR = /^\s*(=\d+|zero|one|two|few|many|other)\s*\{/u;

/**
 * `message` with each ICU plural in it written for `locale`, as in
 * `$localize`:@@id:{${count}:COUNT:, plural, one {# file} other {# files}}``: `$localize` leaves
 * a plural as text with its number put in, and each translation brings the cases its language
 * has. The case is the exact one (`=0`) if there is one, else the one the locale's plural rules
 * pick, else `other`; `#` in it is the number, written as the locale writes numbers.
 */
export function plural(message: string, locale: string): string {
  const rules = new Intl.PluralRules(locale);
  const numbers = new Intl.NumberFormat(locale);
  let written = '';
  let from = 0;
  for (const start of message.matchAll(START)) {
    if (start.index < from) {
      continue;
    }
    const count = Number(start[1]);
    const parsed = cases(message, start.index + start[0].length);
    if (parsed === null) {
      continue;
    }
    const chosen =
      parsed.cases.get(`=${String(count)}`) ??
      parsed.cases.get(rules.select(count)) ??
      parsed.cases.get('other') ??
      '';
    written += message.slice(from, start.index) + chosen.replaceAll('#', numbers.format(count));
    from = parsed.end;
  }
  return written + message.slice(from);
}

/** The cases of a plural from `at` to its closing brace, and where it ends; `null` if unclosed. */
function cases(
  message: string,
  at: number,
): { readonly cases: ReadonlyMap<string, string>; readonly end: number } | null {
  const found = new Map<string, string>();
  let index = at;
  for (;;) {
    const rest = message.slice(index);
    const closing = /^\s*\}/u.exec(rest);
    if (closing !== null) {
      return { cases: found, end: index + closing[0].length };
    }
    const selector = SELECTOR.exec(rest);
    const name = selector?.[1];
    if (selector === null || name === undefined) {
      return null;
    }
    const open = index + selector[0].length;
    const close = closingBrace(message, open);
    if (close === null) {
      return null;
    }
    found.set(name, message.slice(open, close));
    index = close + 1;
  }
}

/** Where the brace that closes the one opened just before `at` is, counting any inside it. */
function closingBrace(message: string, at: number): number | null {
  let depth = 1;
  for (let index = at; index < message.length; index += 1) {
    const char = message.charAt(index);
    depth += char === '{' ? 1 : char === '}' ? -1 : 0;
    if (depth === 0) {
      return index;
    }
  }
  return null;
}
