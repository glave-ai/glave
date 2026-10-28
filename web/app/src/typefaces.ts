/**
 * The typefaces the app draws its pages in (Glaze's `--glz-font-sans` and `--glz-font-mono`),
 * each weight it uses, as `FontFaceSet.load` takes them.
 */
const TYPEFACES = [
  '300 1em "Atkinson Hyperlegible Next"',
  '400 1em "Atkinson Hyperlegible Next"',
  '700 1em "Atkinson Hyperlegible Next"',
  '400 1em "Atkinson Hyperlegible Mono"',
] as const;

/**
 * How long the app waits for them before it draws a page: a few small files from the server it
 * came from. After that it draws in the system's typeface, and swaps theirs in once they come.
 */
const TYPEFACES_LIMIT_MS = 1_000;

/**
 * Loads the app's typefaces from `fonts`, so its first page is drawn in them and no text moves
 * as they swap in, for at most `limit` milliseconds; a typeface that fails to load is left to
 * the system's. Resolves either way.
 */
export async function typefaces(
  fonts: Pick<FontFaceSet, 'load'>,
  limit: number = TYPEFACES_LIMIT_MS,
): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<void>((resolve) => {
    timer = setTimeout(resolve, limit);
  });
  const loaded = Promise.allSettled(TYPEFACES.map((typeface) => fonts.load(typeface)));
  await Promise.race([loaded, late]);
  clearTimeout(timer);
}
