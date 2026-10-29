// Glaze's colour tokens worked out as a browser works them out, from tokens.css itself, for the
// contrast test (tokens/contrast.spec.ts): jsdom computes no colours, and the test must not
// depend on a browser's. It reads exactly what tokens.css writes: hex and `rgb()` colours,
// `light-dark()`, `var()`, `color-mix(in oklch, …)` and relative `oklch(from …)` colours whose
// channels are sums, products, `min()`, `max()` and `clamp()`. Anything else fails loudly.

/** A colour in sRGB, each channel 0 to 1, and its alpha. */
export interface Rgba {
  readonly r: number;
  readonly g: number;
  readonly b: number;
  readonly a: number;
}

/** A colour in OKLCH, lightness 0 to 1, hue in degrees, and its alpha. */
interface Oklch {
  readonly l: number;
  readonly c: number;
  readonly h: number;
  readonly a: number;
}

/** Which of a `light-dark()` pair applies. */
export type Scheme = 'light' | 'dark';

/** Each custom property tokens.css sets on `:root`, by name, as written. */
export function tokensIn(css: string): ReadonlyMap<string, string> {
  const root = /:root\s*\{([\s\S]*?)\n\}/u.exec(css.replace(/\/\*[\s\S]*?\*\//gu, ''))?.[1];
  if (root === undefined) {
    throw new Error('tokens.css sets nothing on :root');
  }
  const tokens = new Map<string, string>();
  for (const [, name = '', value = ''] of root.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/gu)) {
    tokens.set(name, value.replace(/\s+/gu, ' ').trim());
  }
  return tokens;
}

/** The colour `token` comes to in `scheme`. */
export function colorOf(tokens: ReadonlyMap<string, string>, token: string, scheme: Scheme): Rgba {
  return toRgba(parse(tokens, scheme, `var(${token})`));
}

/** `top` drawn over `bottom`, which is opaque. */
export function over(top: Rgba, bottom: Rgba): Rgba {
  const mix = (front: number, back: number): number => front * top.a + back * (1 - top.a);
  return { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a: 1 };
}

/** The contrast ratio of two opaque colours, as WCAG 2.2 defines it: 1 to 21. */
export function contrast(one: Rgba, other: Rgba): number {
  const [lighter, darker] = [luminance(one), luminance(other)].sort((x, y) => y - x);
  return ((lighter ?? 0) + 0.05) / ((darker ?? 0) + 0.05);
}

/** A colour as `#rrggbb`, for a failure's message. */
export function hex(color: Rgba): string {
  const byte = (channel: number): string =>
    Math.round(clamp01(channel) * 255)
      .toString(16)
      .padStart(2, '0');
  return `#${byte(color.r)}${byte(color.g)}${byte(color.b)}`;
}

function luminance({ r, g, b }: Rgba): number {
  const linear = (channel: number): number => {
    const value = clamp01(channel);
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/** The arguments of a function call, split at its own commas. */
function argumentsOf(text: string, name: string): readonly string[] {
  const inner = text.slice(name.length + 1, -1);
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < inner.length; index++) {
    const char = inner.charAt(index);
    depth += char === '(' ? 1 : char === ')' ? -1 : 0;
    if (char === ',' && depth === 0) {
      parts.push(inner.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(inner.slice(start).trim());
  return parts;
}

/** The colour `text` names, in OKLCH. */
function parse(tokens: ReadonlyMap<string, string>, scheme: Scheme, text: string): Oklch {
  const value = text.trim();
  if (value.startsWith('var(')) {
    const name = argumentsOf(value, 'var')[0] ?? '';
    const set = tokens.get(name);
    if (set === undefined) {
      throw new Error(`no token ${name}`);
    }
    return parse(tokens, scheme, set);
  }
  if (value.startsWith('light-dark(')) {
    const [light = '', dark = ''] = argumentsOf(value, 'light-dark');
    return parse(tokens, scheme, scheme === 'light' ? light : dark);
  }
  if (value.startsWith('color-mix(')) {
    return mixed(tokens, scheme, argumentsOf(value, 'color-mix'));
  }
  if (value.startsWith('oklch(from ')) {
    return relative(tokens, scheme, value.slice('oklch(from '.length, -1));
  }
  if (value === 'transparent') {
    return { l: 0, c: 0, h: Number.NaN, a: 0 };
  }
  return toOklch(literal(value));
}

/** A hex or `rgb()` colour. */
function literal(value: string): Rgba {
  const short = /^#([\da-f])([\da-f])([\da-f])$/iu.exec(value);
  const long = /^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/iu.exec(value);
  const digits = short?.slice(1).map((digit) => digit + digit) ?? long?.slice(1);
  if (digits !== undefined) {
    const [r = 0, g = 0, b = 0] = digits.map((pair) => parseInt(pair, 16) / 255);
    return { r, g, b, a: 1 };
  }
  const rgb = /^rgb\((\d+) (\d+) (\d+)(?: \/ (\d+)%)?\)$/u.exec(value);
  if (rgb !== null) {
    const [r = 0, g = 0, b = 0] = rgb.slice(1, 4).map((channel) => Number(channel) / 255);
    return { r, g, b, a: rgb[4] === undefined ? 1 : Number(rgb[4]) / 100 };
  }
  throw new Error(`a colour the contrast test cannot read: ${value}`);
}

/** `color-mix(in oklch, a p, b q)`, its percentages defaulting as CSS's do. */
function mixed(
  tokens: ReadonlyMap<string, string>,
  scheme: Scheme,
  args: readonly string[],
): Oklch {
  const [space, first = '', second = ''] = args;
  if (space !== 'in oklch') {
    throw new Error(`a mix the contrast test cannot read: ${args.join(', ')}`);
  }
  const split = (part: string): { color: Oklch; share: number | null } => {
    const share = /\s(\d+(?:\.\d+)?)%$/u.exec(part);
    const color = share === null ? part : part.slice(0, share.index);
    return { color: parse(tokens, scheme, color), share: share ? Number(share[1]) / 100 : null };
  };
  const one = split(first);
  const two = split(second);
  const q = two.share ?? (one.share === null ? 0.5 : 1 - one.share);
  const p = one.share ?? 1 - q;
  // Premultiplied by alpha, as CSS mixes; a hue that is missing takes the other's.
  const a = one.color.a * p + two.color.a * q;
  const weigh = (x: number, y: number): number =>
    a === 0 ? 0 : (x * one.color.a * p + y * two.color.a * q) / a;
  return {
    l: weigh(one.color.l, two.color.l),
    c: weigh(one.color.c, two.color.c),
    h: hueBetween(one.color.h, two.color.h, q / (p + q)),
    a,
  };
}

/** The shorter way round from hue `from` to `to`, `t` of the way. */
function hueBetween(from: number, to: number, t: number): number {
  if (Number.isNaN(from)) {
    return to;
  }
  if (Number.isNaN(to)) {
    return from;
  }
  const turn = ((((to - from) % 360) + 540) % 360) - 180;
  return from + turn * t;
}

/** `oklch(from <colour> <l> <c> <h>)`, each channel an expression of the colour's. */
function relative(tokens: ReadonlyMap<string, string>, scheme: Scheme, body: string): Oklch {
  const [origin = '', ...channels] = spaced(body);
  const base = parse(tokens, scheme, origin);
  const named = { l: base.l, c: base.c, h: Number.isNaN(base.h) ? 0 : base.h };
  const [l = '', c = '', h = ''] = channels;
  return { l: evaluate(l, named), c: evaluate(c, named), h: evaluate(h, named), a: base.a };
}

/** `text` split at its own spaces, outside brackets. */
function spaced(text: string): readonly string[] {
  const parts: string[] = [];
  let depth = 0;
  let part = '';
  for (const char of text) {
    depth += char === '(' ? 1 : char === ')' ? -1 : 0;
    if (char === ' ' && depth === 0) {
      parts.push(part);
      part = '';
    } else {
      part += char;
    }
  }
  return [...parts, part].filter((each) => each !== '');
}

/** The value of a channel's expression, given the origin's channels. */
function evaluate(text: string, channels: Readonly<Record<string, number>>): number {
  const tokens = text.match(/\d+(?:\.\d+)?(?:deg)?|[a-z]+|[-+*/(),]/gu) ?? [];
  let at = 0;
  const peek = (): string | undefined => tokens[at];
  const take = (): string => tokens[at++] ?? '';
  const sum = (): number => {
    let value = product();
    while (peek() === '+' || peek() === '-') {
      value = take() === '+' ? value + product() : value - product();
    }
    return value;
  };
  const product = (): number => {
    let value = unit();
    while (peek() === '*' || peek() === '/') {
      value = take() === '*' ? value * unit() : value / unit();
    }
    return value;
  };
  const unit = (): number => {
    const token = take();
    if (token === '(') {
      const value = sum();
      take();
      return value;
    }
    if (token === 'min' || token === 'max' || token === 'clamp') {
      take();
      const values = [sum()];
      while (take() === ',') {
        values.push(sum());
      }
      const [low = 0, middle = 0, high = 0] = values;
      return token === 'min'
        ? Math.min(...values)
        : token === 'max'
          ? Math.max(...values)
          : Math.max(low, Math.min(middle, high));
    }
    const channel = channels[token];
    if (channel !== undefined) {
      return channel;
    }
    const number = Number.parseFloat(token);
    if (Number.isNaN(number)) {
      throw new Error(`an expression the contrast test cannot read: ${text}`);
    }
    return number;
  };
  return sum();
}

/** sRGB to OKLCH (Björn Ottosson's OKLab, as CSS Color 4 defines it). */
function toOklch({ r, g, b, a }: Rgba): Oklch {
  const linear = (channel: number): number =>
    channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  const [lr, lg, lb] = [linear(r), linear(g), linear(b)];
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  const lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const x = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const y = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const chroma = Math.hypot(x, y);
  const hue = chroma < 1e-4 ? Number.NaN : (Math.atan2(y, x) * 180) / Math.PI;
  return { l: lightness, c: chroma, h: Number.isNaN(hue) ? hue : (hue + 360) % 360, a };
}

/** OKLCH to sRGB, clipped into its gamut. */
function toRgba({ l, c, h, a }: Oklch): Rgba {
  const angle = ((Number.isNaN(h) ? 0 : h) * Math.PI) / 180;
  const x = c * Math.cos(angle);
  const y = c * Math.sin(angle);
  const lc = (l + 0.3963377774 * x + 0.2158037573 * y) ** 3;
  const mc = (l - 0.1055613458 * x - 0.0638541728 * y) ** 3;
  const sc = (l - 0.0894841775 * x - 1.291485548 * y) ** 3;
  const gamma = (channel: number): number => {
    const value = clamp01(channel);
    return value <= 0.0031308 ? value * 12.92 : 1.055 * value ** (1 / 2.4) - 0.055;
  };
  return {
    r: gamma(4.0767416621 * lc - 3.3077115913 * mc + 0.2309699292 * sc),
    g: gamma(-1.2684380046 * lc + 2.6097574011 * mc - 0.3413193965 * sc),
    b: gamma(-0.0041960863 * lc - 0.7034186147 * mc + 1.707614701 * sc),
    a,
  };
}
