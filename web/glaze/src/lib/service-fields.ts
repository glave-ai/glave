/** One choice of a field: what goes back when it is picked, and what the person reads. */
export interface GlzServiceOption {
  readonly value: string;
  readonly label: string;
}

/** What kind of answer a field takes, and what it starts as, when anything. */
export type GlzServiceFieldKind =
  | { readonly type: 'text'; readonly default?: string | null }
  | { readonly type: 'number'; readonly integer?: boolean; readonly default?: number | null }
  | { readonly type: 'yes_no'; readonly default?: boolean | null }
  | {
      readonly type: 'choice';
      readonly options: readonly GlzServiceOption[];
      readonly multiple?: boolean;
      readonly default?: readonly string[];
    };

/** One thing a connected service asks the person for. */
export interface GlzServiceField {
  /** The name its answer goes back under. */
  readonly name: string;
  readonly label: string;
  readonly description?: string | null;
  readonly required?: boolean;
  readonly kind: GlzServiceFieldKind;
}

/** What a field holds while it is filled in: what was typed, a yes or no, or what was picked. */
export type GlzServiceValue = string | boolean | readonly string[];

/** What a form holds while it is filled in, by each field's name; a field left alone holds none. */
export type GlzServiceValues = Readonly<Record<string, GlzServiceValue | undefined>>;

/** What `field` starts as, if it starts as anything. */
function startOf(kind: GlzServiceFieldKind): GlzServiceValue | undefined {
  switch (kind.type) {
    case 'text':
      return kind.default ?? undefined;
    case 'number':
      return kind.default === null || kind.default === undefined ? undefined : String(kind.default);
    case 'yes_no':
      return kind.default ?? undefined;
    case 'choice':
      return kind.multiple === true ? (kind.default ?? []) : kind.default?.[0];
  }
}

/** What `fields` start as. */
export function startingValues(fields: readonly GlzServiceField[]): GlzServiceValues {
  return Object.fromEntries(fields.map((field) => [field.name, startOf(field.kind)]));
}

/** What `value` sends for a field of `kind`; `undefined` when it holds no answer, or a wrong one. */
function sent(kind: GlzServiceFieldKind, value: GlzServiceValue | undefined): unknown {
  if (kind.type === 'number') {
    const typed = typeof value === 'string' ? value.trim() : '';
    const number = Number(typed);
    const fits = typed !== '' && Number.isFinite(number);
    return fits && (kind.integer !== true || Number.isInteger(number)) ? number : undefined;
  }
  if (kind.type === 'text') {
    const typed = typeof value === 'string' ? value.trim() : '';
    return typed === '' ? undefined : typed;
  }
  if (Array.isArray(value) && value.length === 0) {
    return undefined;
  }
  return value === '' ? undefined : value;
}

/**
 * What `values` answer `fields` with, by each field's name, leaving out each optional field left
 * empty; `null` while a required one is empty or a number cannot be read.
 */
export function answerOf(
  fields: readonly GlzServiceField[],
  values: GlzServiceValues,
): Readonly<Record<string, unknown>> | null {
  // By entries, so a field named `__proto__` is a field like any other.
  const answer: [string, unknown][] = [];
  for (const field of fields) {
    const value = values[field.name];
    const given = sent(field.kind, value);
    const typed = typeof value === 'string' && value.trim() !== '';
    if (
      given === undefined &&
      (field.required === true || (field.kind.type === 'number' && typed))
    ) {
      return null;
    }
    if (given !== undefined) {
      answer.push([field.name, given]);
    }
  }
  return Object.fromEntries(answer);
}

/** Whether `a` and `b` ask the same: the same fields, kinds and starts, in order. */
export function sameFields(a: readonly GlzServiceField[], b: readonly GlzServiceField[]): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
