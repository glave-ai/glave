/** A plural: its number, as `$localize` put it in, then its cases, each in braces. */
const PLURAL =
  /\{\s*(-?\d+(?:\.\d+)?)\s*,\s*plural\s*,((?:\s*(?:=\d+|zero|one|two|few|many|other)\s*\{[^{}]*\})+)\s*\}/gu;

/** One case of a plural: `one`, `other`, or an exact `=0`, and its words. */
const CASE = /(=\d+|zero|one|two|few|many|other)\s*\{([^{}]*)\}/gu;

/**
 * `message` with each ICU plural in it written for `locale`, as in
 * `$localize`:@@id:{${count}:COUNT:, plural, one {# file} other {# files}}``: `$localize` leaves
 * a plural as text with its number put in, and each translation brings the cases its language
 * has. The case is the exact one (`=0`) if there is one, else the one the locale's plural rules
 * pick, else `other`; `#` in it is the number, written as the locale writes numbers. A case
 * holds no braces, so no placeholder and no plural (`npm run build` checks): their values are
 * put in before the plural is read.
 */
export function plural(message: string, locale: string): string {
  const rules = new Intl.PluralRules(locale);
  const numbers = new Intl.NumberFormat(locale);
  return message.replace(PLURAL, (_plural, number: string, body: string) => {
    const count = Number(number);
    const cases = new Map([...body.matchAll(CASE)].map(([, name, words]) => [name, words]));
    const chosen =
      cases.get(`=${number}`) ?? cases.get(rules.select(count)) ?? cases.get('other') ?? '';
    return chosen.replaceAll('#', numbers.format(count));
  });
}
