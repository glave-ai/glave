/**
 * The last block of markdown still being written, settled so it renders now as it will once
 * finished: no `**` that turns bold a moment later, no half-typed link or list marker. Only
 * the block's last line changes, and only while it streams.
 */

/**
 * A last line that starts a block not yet decided: a list marker (with a task box half typed,
 * or a number that may become one), `#`, `>`, a table row's `|`, a rule or a fence.
 */
const UNDECIDED =
  /^[ \t]*(?:(?:[-+*]|\d{1,9}[.)]?)(?:[ \t]+\[[ xX]?\]?)?|#{1,6}|>|\||[-=_*]+|`{1,2}|~{1,2})[ \t]*$/u;

/** A fence line, which opens or closes a code block. */
const FENCE = /^[ \t]*(?:`{3,}|~{3,})/u;

/** A table row: its header shows only once its delimiter row makes it a table. */
const ROW = /^[ \t]*\|/u;

/**
 * Inline markers that open and close: strong, emphasis and GFM strikethrough; and a lone `~`,
 * which matters only at the end of the line, as half of a `~~`.
 */
const MARKERS = ['**', '__', '~~', '*', '_', '~'] as const;

type Marker = (typeof MARKERS)[number];

interface Open {
  readonly marker: Marker;
  /** Where it starts in the line. */
  readonly at: number;
}

/** A link or image being written: `[text](url…` with no `)` yet. */
interface PartialLink {
  readonly at: number;
  readonly image: boolean;
  readonly text: string;
}

function isSpace(char: string | undefined): boolean {
  return char === undefined || /\s/u.test(char);
}

function isWordChar(char: string | undefined): boolean {
  return char !== undefined && /[\p{L}\p{N}]/u.test(char);
}

/** The marker at `index`, longest first. */
function markerAt(line: string, index: number): Marker | null {
  return MARKERS.find((marker) => line.startsWith(marker, index)) ?? null;
}

/**
 * Whether `marker` at `index` can open: text follows it, or nothing yet (the line ends there),
 * and `_` is not inside a word.
 */
function canOpen(line: string, index: number, marker: Marker): boolean {
  const after = line[index + marker.length];
  const before = line[index - 1];
  if (marker === '~') {
    return after === undefined;
  }
  return (
    (after === undefined || !isSpace(after)) && !(marker.startsWith('_') && isWordChar(before))
  );
}

/** Whether `marker` at `index` can close: it follows something, and `_` not inside a word. */
function canClose(line: string, index: number, marker: Marker): boolean {
  const after = line[index + marker.length];
  const before = line[index - 1];
  return marker !== '~' && !isSpace(before) && !(marker.startsWith('_') && isWordChar(after));
}

/** The line's open markers, its open code span and its link in progress, scanned left to right. */
interface Scan {
  readonly open: readonly Open[];
  /** Where an open code span starts, or -1. */
  readonly codeAt: number;
  readonly codeTicks: number;
  readonly link: PartialLink | null;
}

/** Where the link that `]` at `index` closes starts, and whether it is an image. */
function linkStart(line: string, index: number): { at: number; image: boolean } | null {
  const at = line.lastIndexOf('[', index);
  if (at === -1) {
    return null;
  }
  return line[at - 1] === '!' ? { at: at - 1, image: true } : { at, image: false };
}

function scan(line: string): Scan {
  const open: Open[] = [];
  let codeAt = -1;
  let codeTicks = 0;
  let link: PartialLink | null = null;
  let index = 0;
  while (index < line.length) {
    const char = line[index];
    if (char === '`') {
      const ticks = /^`+/u.exec(line.slice(index))?.[0].length ?? 1;
      if (codeAt === -1) {
        codeAt = index;
        codeTicks = ticks;
      } else if (ticks === codeTicks) {
        codeAt = -1;
      }
      index += ticks;
      continue;
    }
    if (codeAt !== -1) {
      index += 1;
      continue;
    }
    if (char === '\\') {
      index += 2;
      continue;
    }
    if (link !== null) {
      if (char === ')') {
        link = null;
      }
      index += 1;
      continue;
    }
    if (char === ']' && line[index + 1] === '(') {
      const start = linkStart(line, index);
      if (start !== null) {
        const textStart = start.at + (start.image ? 2 : 1);
        link = { ...start, text: line.slice(textStart, index) };
        // The link's own markers close inside it, so they leave the stack with it.
        while ((open.at(-1)?.at ?? -1) > start.at) {
          open.pop();
        }
      }
      index += 2;
      continue;
    }
    const marker = markerAt(line, index);
    if (marker === null) {
      index += 1;
      continue;
    }
    const top = open.at(-1);
    if (top?.marker === marker && canClose(line, index, marker)) {
      open.pop();
    } else if (canOpen(line, index, marker)) {
      open.push({ marker, at: index });
    }
    index += marker.length;
  }
  return { open, codeAt, codeTicks, link };
}

/**
 * The last line, settled: a link still being written shows its text (an image, nothing); an
 * open code span or marker with nothing after it goes, and one with text after it is closed.
 */
function settleLine(line: string): string {
  const { open, codeAt, codeTicks, link } = scan(line);
  if (link !== null) {
    const shown = link.image ? '' : link.text;
    return settleLine(`${line.slice(0, link.at)}${shown}`);
  }
  if (codeAt !== -1) {
    const inside = line.slice(codeAt + codeTicks);
    return inside.trim() === ''
      ? line.slice(0, codeAt).trimEnd()
      : `${line}${'`'.repeat(codeTicks)}`;
  }
  let settled = line;
  let closers = '';
  for (const marker of [...open].reverse()) {
    const after = settled.slice(marker.at + marker.marker.length);
    if (after.trim() === '') {
      settled = settled.slice(0, marker.at);
    } else {
      closers = `${closers}${marker.marker}`;
    }
  }
  return closers === '' ? settled : `${settled.trimEnd()}${closers}`;
}

/** Whether `lines` end inside a fenced code block: an odd number of fence lines. */
function inFence(lines: readonly string[]): boolean {
  return lines.filter((line) => FENCE.test(line)).length % 2 === 1;
}

/**
 * `raw`, the markdown of the last block while it is still being written, settled to render as
 * it will when finished. `paragraph` says marked read it as a paragraph, so table rows at its
 * end are a table still waiting for its delimiter row: they show once it comes.
 */
export function settleTail(raw: string, paragraph: boolean): string {
  const lines = raw.split('\n');
  const last = lines.at(-1) ?? '';
  if (inFence(lines.slice(0, -1)) || FENCE.test(last)) {
    return raw;
  }
  if (UNDECIDED.test(last)) {
    lines.pop();
    return lines.join('\n');
  }
  if (paragraph && ROW.test(last)) {
    while (lines.length > 0 && ROW.test(lines.at(-1) ?? '')) {
      lines.pop();
    }
    return lines.join('\n');
  }
  lines[lines.length - 1] = settleLine(last).trimEnd();
  return lines.join('\n');
}
