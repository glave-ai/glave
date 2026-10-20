/** One choice in a menu or picker: what it stands for, and what people read. */
export interface GlzOption {
  readonly value: string;
  readonly label: string;
}

/**
 * The options whose label, or the hint under it, contains `query`, ignoring case and accents,
 * in their order: a model is found by its name or by the service in its hint. An empty query
 * keeps them all.
 */
export function filterOptions<T extends GlzOption & { readonly hint?: string }>(
  options: readonly T[],
  query: string,
): readonly T[] {
  const wanted = fold(query.trim());
  return wanted === ''
    ? options
    : options.filter((option) => fold(`${option.label}\n${option.hint ?? ''}`).includes(wanted));
}

function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase();
}
