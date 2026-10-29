import tokensCss from '../../styles/tokens.css' with { loader: 'text' };
import { type Rgba, type Scheme, colorOf, contrast, hex, over, tokensIn } from '../testing/colors';

/** WCAG 2.2's least contrast: text (1.4.3), and what is not text, focus included (1.4.11). */
const TEXT = 4.5;
const NOT_TEXT = 3;

/**
 * One pair the app draws: a colour, the surfaces under it from the page up (the first opaque,
 * any above it see-through, drawn over it), the least contrast it needs, and what it is.
 */
interface Pair {
  readonly color: string;
  readonly on: readonly (readonly string[])[];
  readonly least: number;
  readonly what: string;
}

/** The panes text sits on, and the boxes on them. */
const PANES = ['--glz-surface-page', '--glz-surface-sidebar'];
const BOXES = [
  '--glz-surface-sunken',
  '--glz-surface-card',
  '--glz-surface-raised',
  '--glz-surface-overlay',
  '--glz-surface-bubble',
];
const EVERY_SURFACE = [...PANES, ...BOXES].map((surface) => [surface]);
/** A row under the pointer, or the one chosen, in a side pane or a menu. */
const ROWS = [
  ...[...PANES, '--glz-surface-card', '--glz-surface-overlay'].map((surface) => [
    surface,
    '--glz-surface-hover',
  ]),
  ['--glz-surface-selected'],
];
/** Where status shows: the page, the side nav, a card, a turn's receipt. */
const STATUS_ON = [
  '--glz-surface-page',
  '--glz-surface-sidebar',
  '--glz-surface-card',
  '--glz-surface-sunken',
].map((surface) => [surface]);
/** Where controls sit: the page, the side nav, a card, a dialog or menu. */
const CONTROLS_ON = [
  '--glz-surface-page',
  '--glz-surface-sidebar',
  '--glz-surface-card',
  '--glz-surface-overlay',
].map((surface) => [surface]);

const PAIRS: readonly Pair[] = [
  ...['--glz-color-text', '--glz-color-text-secondary', '--glz-color-text-muted'].map((color) => ({
    color,
    on: [...EVERY_SURFACE, ...ROWS],
    least: TEXT,
    what: 'text',
  })),
  { color: '--glz-accent-text', on: EVERY_SURFACE, least: TEXT, what: 'a link, a step at work' },
  ...['--glz-color-success', '--glz-color-warning', '--glz-color-danger'].map((color) => ({
    color,
    on: STATUS_ON,
    least: TEXT,
    what: 'status as text',
  })),
  ...['--glz-color-added', '--glz-color-removed'].map((color) => ({
    color,
    on: [['--glz-surface-card'], ['--glz-surface-sidebar']],
    least: TEXT,
    what: 'lines a change added or removed',
  })),
  ...['--glz-color-success-soft', '--glz-color-warning-soft', '--glz-color-danger-soft'].map(
    (soft) => ({
      color: '--glz-color-text',
      on: [...PANES, '--glz-surface-card', '--glz-surface-overlay'].map((surface) => [
        surface,
        soft,
      ]),
      least: TEXT,
      what: 'text on a tone’s soft fill: a badge, a changed line',
    }),
  ),
  {
    color: '--glz-color-text',
    on: [...PANES, '--glz-surface-card'].map((surface) => [surface, '--glz-accent-soft']),
    least: TEXT,
    what: 'text on the accent’s soft fill: selected text, an avatar',
  },
  {
    color: '--glz-on-accent',
    on: [['--glz-accent'], ['--glz-accent-hover']],
    least: TEXT,
    what: 'a primary button’s label',
  },
  {
    color: '--glz-color-on-danger',
    on: [['--glz-color-danger'], ['--glz-color-danger-hover']],
    least: TEXT,
    what: 'a danger button’s label',
  },
  {
    color: '--glz-color-on-inverse',
    on: [['--glz-surface-inverse']],
    least: TEXT,
    what: 'a tooltip',
  },
  {
    color: '--glz-selected-marker',
    on: [...CONTROLS_ON, ['--glz-surface-selected']],
    least: NOT_TEXT,
    what: 'the marker beside what is chosen',
  },
  {
    color: '--glz-color-focus',
    on: [...EVERY_SURFACE, ['--glz-surface-selected']],
    least: NOT_TEXT,
    what: 'the focus ring',
  },
  ...['--glz-accent', '--glz-color-danger'].map((color) => ({
    color,
    on: CONTROLS_ON,
    least: NOT_TEXT,
    what: 'a filled button, a chosen box or switch',
  })),
  {
    color: '--glz-color-success',
    on: [['--glz-surface-card']],
    least: NOT_TEXT,
    what: 'a connected service’s dot',
  },
];

const TOKENS = tokensIn(tokensCss);

/** The surfaces `layers` come to, drawn from the first up. */
function surfaceOf(layers: readonly string[], scheme: Scheme): Rgba {
  const [first = '', ...rest] = layers;
  const base = colorOf(TOKENS, first, scheme);
  if (base.a !== 1) {
    throw new Error(`${first} is see-through, so it cannot be the first surface`);
  }
  return rest.reduce((under, layer) => over(colorOf(TOKENS, layer, scheme), under), base);
}

/** Each pair below its least contrast in `scheme`, as a line saying by how much. */
function failing(scheme: Scheme): readonly string[] {
  return PAIRS.flatMap(({ color, on, least, what }) =>
    on.flatMap((layers) => {
      const surface = surfaceOf(layers, scheme);
      const drawn = over(colorOf(TOKENS, color, scheme), surface);
      const ratio = contrast(drawn, surface);
      return ratio >= least
        ? []
        : [
            `${what}: ${color} ${hex(drawn)} on ${layers.join(' + ')} ${hex(surface)} is ` +
              `${ratio.toFixed(2)}:1, under ${String(least)}:1`,
          ];
    }),
  );
}

describe('the colour tokens', () => {
  it('reads what tokens.css writes, as a browser computes it', () => {
    expect(hex(colorOf(TOKENS, '--glz-color-text', 'light'))).toBe('#1b1c1d');
    expect(hex(colorOf(TOKENS, '--glz-color-text', 'dark'))).toBe('#ececec');
    expect(hex(colorOf(TOKENS, '--glz-on-accent', 'light'))).toBe('#ffffff');
    expect(hex(colorOf(TOKENS, '--glz-on-accent', 'dark'))).toBe('#000000');
    expect(colorOf(TOKENS, '--glz-accent-soft', 'light').a).toBeCloseTo(0.12);
    expect(contrast(colorOf(TOKENS, '--glz-on-accent', 'light'), { r: 0, g: 0, b: 0, a: 1 })).toBe(
      21,
    );
  });

  for (const scheme of ['light', 'dark'] as const) {
    it(`meets WCAG 2.2 AA contrast in every pair the app draws, in ${scheme}`, () => {
      expect(failing(scheme)).toEqual([]);
    });
  }
});
