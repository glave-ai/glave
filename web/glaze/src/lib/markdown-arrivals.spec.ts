import { GlzMarkdownArrivals } from './markdown-arrivals';

const SLOW = 320;

/** A host showing `html`, and its arrivals at a time the test moves. */
function setUp(html: string): {
  readonly host: HTMLElement;
  readonly arrivals: GlzMarkdownArrivals;
  readonly at: (ms: number) => void;
} {
  const host = document.createElement('div');
  host.innerHTML = html;
  let now = 1000;
  const arrivals = new GlzMarkdownArrivals(host, () => now);
  return {
    host,
    arrivals,
    at: (ms) => {
      now = ms;
    },
  };
}

/** Each piece fading in: its text and where its fade has got to. */
function fading(host: HTMLElement): readonly (readonly [string | null, string])[] {
  return Array.from(host.querySelectorAll<HTMLElement>('.glz-markdown-arrived'), (span) => [
    span.textContent,
    span.style.animationDelay,
  ]);
}

describe('GlzMarkdownArrivals', () => {
  it('fades in only what arrives after the first render, never what was there', () => {
    const { host, arrivals, at } = setUp('<p>The lock</p>');
    arrivals.rendered(SLOW);
    expect(fading(host)).toEqual([]);
    at(1010);
    host.innerHTML = '<p>The lock is taken</p>';
    arrivals.rendered(SLOW);
    expect(fading(host)).toEqual([[' is taken', '0ms']]);
    // The text reads as it did: nothing added or moved.
    expect(host.textContent).toBe('The lock is taken');
  });

  it('wraps a piece still fading again where it got to when its block is redone', () => {
    const { host, arrivals, at } = setUp('<p>The lock</p>');
    arrivals.rendered(SLOW);
    at(1010);
    host.innerHTML = '<p>The lock is</p>';
    arrivals.rendered(SLOW);
    at(1110);
    host.innerHTML = '<p>The lock is <strong>taken</strong></p>';
    arrivals.rendered(SLOW);
    expect(fading(host)).toEqual([
      [' is', '-100ms'],
      [' ', '0ms'],
      ['taken', '0ms'],
    ]);
    // Once its fade is over, a piece is left as plain text.
    at(1400);
    host.innerHTML = '<p>The lock is <strong>taken</strong> twice</p>';
    arrivals.rendered(SLOW);
    expect(fading(host)).toEqual([
      [' ', '-290ms'],
      ['taken', '-290ms'],
      [' twice', '0ms'],
    ]);
  });

  it('leaves a block not redone as it is, its pieces fading on', () => {
    const { host, arrivals, at } = setUp('<div><p>One</p></div><div><p>T</p></div>');
    arrivals.rendered(SLOW);
    at(1010);
    const tail = host.lastElementChild;
    if (tail !== null) {
      tail.innerHTML = '<p>Two</p>';
    }
    arrivals.rendered(SLOW);
    const span = host.querySelector('.glz-markdown-arrived');
    at(1020);
    host.insertAdjacentHTML('beforeend', '<div><p>Three</p></div>');
    arrivals.rendered(SLOW);
    expect(host.querySelector('.glz-markdown-arrived')).toBe(span);
    expect(fading(host)).toEqual([
      ['wo', '0ms'],
      ['Three', '0ms'],
    ]);
  });

  it('never counts a code block’s copy button as text', () => {
    const { host, arrivals, at } = setUp('<pre><code>cargo</code></pre>');
    arrivals.rendered(SLOW);
    at(1010);
    host.innerHTML =
      '<div><glz-copy-button>Copy code</glz-copy-button></div><pre><code>cargo test</code></pre>';
    arrivals.rendered(SLOW);
    expect(fading(host)).toEqual([[' test', '0ms']]);
  });

  it('moves nothing when the person asks for less motion, or the text is not streaming', () => {
    const { host, arrivals, at } = setUp('<p>The lock</p>');
    arrivals.rendered(0);
    at(1010);
    host.innerHTML = '<p>The lock is taken</p>';
    arrivals.rendered(0);
    expect(fading(host)).toEqual([]);
  });
});
