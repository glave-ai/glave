/** The class each newly arrived piece of streaming text is wrapped in while it fades in. */
const ARRIVED = 'glz-markdown-arrived';

/** What is not the text itself: a code block's copy button, which Glaze adds. */
const NOT_TEXT = 'glz-copy-button';

/** A piece of the text that arrived at once: its place in the text, and when, in ms. */
interface Arrival {
  readonly from: number;
  readonly to: number;
  readonly at: number;
}

/** The text shown in `host`, each node with where it starts, leaving out what is not text. */
function textNodes(host: HTMLElement): {
  readonly text: string;
  readonly nodes: readonly [Text, number][];
} {
  const walker = host.ownerDocument.createTreeWalker(
    host,
    NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
    {
      acceptNode: (node) =>
        node instanceof Element && node.localName === NOT_TEXT
          ? NodeFilter.FILTER_REJECT
          : node instanceof Text
            ? NodeFilter.FILTER_ACCEPT
            : NodeFilter.FILTER_SKIP,
    },
  );
  const nodes: [Text, number][] = [];
  let text = '';
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (node instanceof Text) {
      nodes.push([node, text.length]);
      text += node.data;
    }
  }
  return { text, nodes };
}

/** How many characters `one` and `other` share from their start. */
function shared(one: string, other: string): number {
  const most = Math.min(one.length, other.length);
  let index = 0;
  while (index < most && one.charCodeAt(index) === other.charCodeAt(index)) {
    index += 1;
  }
  return index;
}

/**
 * Streaming markdown's newly arrived text, faded in: after each render of `host`, the text past
 * what it showed before is a piece that arrived, wrapped in a span that fades in over `duration`
 * ms, timed from its arrival. A block rendered again, as the growing last one is, has its pieces
 * still fading wrapped again, each where its fade has got to, so none starts over, and once a
 * piece's fade is over it is left alone. What the first render shows, the text already there,
 * never fades in. Only text is wrapped, never a whole element, and the spans are plain inline
 * elements: the text reads, selects and copies as before.
 */
export class GlzMarkdownArrivals {
  /** The text shown at the last render; `null` before the first. */
  private shown: string | null = null;
  private arrivals: Arrival[] = [];

  constructor(
    private readonly host: HTMLElement,
    private readonly now: () => number,
  ) {}

  /** After a render: fades in what arrived, over `duration` ms; nothing at all with 0. */
  rendered(duration: number): void {
    const { text, nodes } = textNodes(this.host);
    const before = this.shown;
    this.shown = text;
    if (before === null || duration <= 0) {
      this.arrivals = [];
      return;
    }
    const now = this.now();
    const kept = shared(before, text);
    // What changed of the text before is shown again as it is now, not as a piece that arrived.
    this.arrivals = this.arrivals
      .filter((arrival) => now - arrival.at < duration && arrival.from < kept)
      .map((arrival) => ({ ...arrival, to: Math.min(arrival.to, kept) }));
    if (text.length > kept) {
      this.arrivals.push({ from: kept, to: text.length, at: now });
    }
    // From the end, so a node split for a later piece keeps the start the earlier ones read.
    for (const arrival of [...this.arrivals].reverse()) {
      wrap(nodes, arrival, now - arrival.at);
    }
  }
}

/** Wraps the text of `arrival` in `nodes`, each part not wrapped yet, `elapsed` ms into its fade. */
function wrap(nodes: readonly [Text, number][], arrival: Arrival, elapsed: number): void {
  for (const [node, start] of nodes) {
    const end = start + node.data.length;
    if (end <= arrival.from || start >= arrival.to) {
      continue;
    }
    // Already fading, or only the space between blocks, which shows nothing to fade.
    if (node.parentElement?.classList.contains(ARRIVED) === true || node.data.trim() === '') {
      continue;
    }
    let piece = node;
    if (arrival.from > start) {
      piece = piece.splitText(arrival.from - start);
    }
    if (arrival.to < end) {
      piece.splitText(arrival.to - Math.max(start, arrival.from));
    }
    const span = piece.ownerDocument.createElement('span');
    span.className = ARRIVED;
    // Where its fade has got to, through the style object, which the page's policy allows.
    span.style.animationDelay = `${String(-elapsed)}ms`;
    piece.replaceWith(span);
    span.append(piece);
  }
}
