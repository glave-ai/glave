import { Lexer, type Links, type MarkedOptions, Parser, type Token } from 'marked';

import { type GlzHighlight, type GlzMarkdownLabels, GlzMarkdownRenderer } from './markdown-html';
import { settleTail } from './markdown-tail';

/** GitHub-flavoured: tables, task lists, strikethrough, bare links; a newline breaks the line. */
const OPTIONS: MarkedOptions = { gfm: true, breaks: true };

/** Blocks that render nothing: blank lines, and link definitions (the links carry them). */
const SILENT: ReadonlySet<string> = new Set(['space', 'def']);

/** How the text is rendered, besides the text itself. */
export interface GlzMarkdownOptions {
  /** Whether the text is still being written, so its last block is half-written. */
  readonly streaming: boolean;
  readonly labels: GlzMarkdownLabels;
  readonly highlight: GlzHighlight | null;
}

/** One top-level block of the text, rendered. */
export interface GlzMarkdownBlock {
  /** Its markdown. */
  readonly raw: string;
  /** Whether it was rendered as the half-written end of a streaming text. */
  readonly tail: boolean;
  /** Its HTML, safe to sanitize and show (`GlzMarkdownRenderer`). */
  readonly html: string;
  /** The languages its code names. */
  readonly languages: readonly string[];
}

/** Markdown rendered block by block, so the next render redoes only what changed. */
export interface GlzMarkdown {
  readonly blocks: readonly GlzMarkdownBlock[];
  /** Every language its code names, for a highlighter to load. */
  readonly languages: readonly string[];
  /** What every block's HTML depends on besides its own markdown. */
  readonly options: GlzMarkdownOptions;
  /** The text's link definitions, as a reference link anywhere resolves them. */
  readonly links: string;
}

function rendered(
  tokens: Token[],
  raw: string,
  tail: boolean,
  options: GlzMarkdownOptions,
): GlzMarkdownBlock {
  const renderer = new GlzMarkdownRenderer(options.labels, options.highlight);
  const html = new Parser({ ...OPTIONS, renderer }).parse(tokens);
  return { raw, tail, html, languages: [...renderer.languages] };
}

/** The half-written last block, settled (`settleTail`) and read again with the text's links. */
function renderedTail(token: Token, links: Links, options: GlzMarkdownOptions): GlzMarkdownBlock {
  const settled =
    token.type === 'code' || token.type === 'html'
      ? token.raw
      : settleTail(token.raw, token.type === 'paragraph');
  if (settled === token.raw) {
    return rendered([token], token.raw, true, options);
  }
  const lexer = new Lexer(OPTIONS);
  Object.assign(lexer.tokens.links, links);
  return rendered(lexer.lex(settled), token.raw, true, options);
}

function sameOptions(one: GlzMarkdownOptions, other: GlzMarkdownOptions): boolean {
  return one.labels === other.labels && one.highlight === other.highlight;
}

/**
 * `text` as HTML, one top-level block at a time. Given the `previous` render of the same text
 * as it grows, each block whose markdown, links and options are unchanged is reused as it
 * was, so only the growing last block is rendered again. While `streaming`, that last block
 * renders as it will once finished (`settleTail`): half-written markdown never flickers.
 */
export function renderMarkdown(
  text: string,
  options: GlzMarkdownOptions,
  previous: GlzMarkdown | null = null,
): GlzMarkdown {
  const tokens = Lexer.lex(text, OPTIONS);
  const links = JSON.stringify(tokens.links);
  const reusable =
    previous !== null && previous.links === links && sameOptions(previous.options, options)
      ? previous.blocks
      : [];
  const shown = tokens.filter((token) => !SILENT.has(token.type));
  const last = tokens.at(-1);
  const blocks = shown
    .map((token, index) => {
      const tail = options.streaming && token === last;
      const before = reusable[index];
      if (before?.raw === token.raw && before.tail === tail) {
        return before;
      }
      return tail
        ? renderedTail(token, tokens.links, options)
        : rendered([token], token.raw, false, options);
    })
    // A tail settled to nothing (a list marker alone) shows nothing yet.
    .filter((block) => !block.tail || block.html !== '');
  const languages = [...new Set(blocks.flatMap((block) => block.languages))];
  return { blocks, languages, options, links };
}
