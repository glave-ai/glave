/** One choice in a combobox or select: what it stands for, and what people read. */
export interface GlzOption {
  readonly value: string;
  readonly label: string;
}

/**
 * The options whose label contains `query`, ignoring case and accents, in their order. An
 * empty query keeps them all.
 */
export function filterOptions(options: readonly GlzOption[], query: string): readonly GlzOption[] {
  const wanted = fold(query.trim());
  return wanted === '' ? options : options.filter((option) => fold(option.label).includes(wanted));
}

/** The label of the option with `value`, or `''`. */
export function labelOf(options: readonly GlzOption[], value: string | null): string {
  return options.find((option) => option.value === value)?.label ?? '';
}

function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase();
}
