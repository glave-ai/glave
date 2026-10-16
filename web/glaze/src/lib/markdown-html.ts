import { Renderer, type Tokens } from 'marked';

/** The words markdown adds around what it renders, for readers and screen readers. */
export interface GlzMarkdownLabels {
  /** After every link's text, for screen readers: links open in a new tab. */
  readonly newTab: string;
  /** Before an image's description: images show as links to them, never fetched. */
  readonly image: string;
  /** A task list's checked and unchecked boxes. */
  readonly done: string;
  readonly notDone: string;
  /** A code block's copy button. */
  readonly copyCode: string;
}

export const GLZ_MARKDOWN_LABELS: GlzMarkdownLabels = {
  newTab: '(opens in a new tab)',
  image: 'Image',
  done: 'Done',
  notDone: 'Not done',
  copyCode: 'Copy code',
};

/** Highlights `code` written in `language` as HTML, or `null` when it does not know it. */
export type GlzHighlight = (code: string, language: string) => string | null;

/** Where a link may go: the web and email. Anything else shows as its text. */
const LINKED = new Set(['http:', 'https:', 'mailto:']);

/** Where an image may be linked to: the web. */
const IMAGED = new Set(['http:', 'https:']);

/** A code block's language as a class can carry it, or `''`. */
const LANGUAGE = /^[\w+#.-]{1,32}$/u;

const ENTITIES: Readonly<Record<string, string>> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

/** `text` with every character HTML gives meaning to escaped. */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/gu, (char) => ENTITIES[char] ?? char);
}

/** `text` escaped, but for character references it already holds (`&amp;`, `&#169;`). */
function escapeText(text: string): string {
  return text.replace(/&(?!#?\w+;)|[<>"']/gu, (char) => ENTITIES[char] ?? char);
}

/** `href` as an absolute URL with one of `protocols`, or `null`. */
function allowedUrl(href: string, protocols: ReadonlySet<string>): string | null {
  try {
    const url = new URL(href);
    return protocols.has(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}

/**
 * Markdown as HTML that loads nothing and runs nothing, whatever the text says. Raw HTML shows
 * as text. An image is never an `<img>`: on the web it is a link to it, else its description,
 * so no reply can make the browser fetch an address (with a secret in it). Links open in a new
 * tab, without the opener or a referrer, and say so; only web and email addresses link. Code
 * blocks carry their language and a head for a copy button, and are highlighted when
 * `highlight` knows the language. Task list boxes are named images, not inputs.
 */
export class GlzMarkdownRenderer extends Renderer {
  /** The languages code blocks named, once their opening line was complete. */
  readonly languages = new Set<string>();
  private linkDepth = 0;

  constructor(
    private readonly labels: GlzMarkdownLabels,
    private readonly highlight: GlzHighlight | null,
  ) {
    super();
  }

  override html({ text, block }: Tokens.HTML | Tokens.Tag): string {
    return block ? `<p class="glz-markdown-raw">${escapeHtml(text)}</p>` : escapeHtml(text);
  }

  override text(token: Tokens.Text | Tokens.Escape): string {
    // marked leaves text inside raw HTML unescaped; here nothing is.
    return 'tokens' in token && token.tokens !== undefined
      ? this.parser.parseInline(token.tokens)
      : escapeText(token.text);
  }

  override link({ href, title, text, tokens, autolink }: Tokens.Link): string {
    this.linkDepth += 1;
    const inner = autolink === true ? escapeHtml(text) : this.parser.parseInline(tokens);
    this.linkDepth -= 1;
    const url = allowedUrl(href, LINKED);
    return url === null ? inner : this.anchor(url, inner, title ?? '', '');
  }

  override image({ href, title, text, tokens }: Tokens.Image): string {
    const described = this.parser.parseInline(tokens, this.parser.textRenderer).trim();
    const words = described === '' ? text.trim() : described;
    const label = escapeHtml(words === '' ? this.labels.image : `${this.labels.image}: ${words}`);
    const url = allowedUrl(href, IMAGED);
    return url === null || this.linkDepth > 0
      ? label
      : this.anchor(url, label, title ?? '', ' class="glz-markdown-image"');
  }

  override code({ text, lang, raw }: Tokens.Code): string {
    const named = (lang ?? '').trim().split(/\s/u)[0] ?? '';
    const language = LANGUAGE.test(named) ? named.toLowerCase() : '';
    if (language !== '' && raw.includes('\n')) {
      this.languages.add(language);
    }
    const code = text.replace(/\n$/u, '');
    const highlighted = language === '' ? null : (this.highlight?.(code, language) ?? null);
    const name =
      language === '' ? '' : `<span class="glz-markdown-language">${escapeHtml(language)}</span>`;
    const kind = language === '' ? '' : ` class="language-${escapeHtml(language)}"`;
    return (
      `<div class="glz-markdown-code"><div class="glz-markdown-code-head">${name}</div>` +
      `<pre tabindex="0"><code${kind}>${highlighted ?? escapeHtml(code)}</code></pre></div>\n`
    );
  }

  override checkbox({ checked }: Tokens.Checkbox): string {
    const state = checked ? ' glz-markdown-task-done' : '';
    const label = escapeHtml(checked ? this.labels.done : this.labels.notDone);
    return `<span class="glz-markdown-task${state}" role="img" aria-label="${label}"></span> `;
  }

  override table(token: Tokens.Table): string {
    return `<div class="glz-markdown-table" tabindex="0">${super.table(token)}</div>\n`;
  }

  private anchor(url: string, inner: string, title: string, extra: string): string {
    const titled = title === '' ? '' : ` title="${escapeHtml(title)}"`;
    const note = `<span class="glz-visually-hidden"> ${escapeHtml(this.labels.newTab)}</span>`;
    return (
      `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer"${titled}${extra}>` +
      `${inner}${note}</a>`
    );
  }
}
