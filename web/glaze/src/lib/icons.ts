import { InjectionToken, type Provider, inject } from '@angular/core';
import type { IconNode } from 'lucide';

/** An icon's drawing in Lucide's format: SVG child elements and their attributes. */
export type GlzIconData = IconNode;

/** Icons registered by name, for `<glz-icon name="…">`. */
export const GLZ_ICONS = new InjectionToken<ReadonlyMap<string, GlzIconData>>('GLZ_ICONS', {
  providedIn: 'root',
  factory: () => new Map(),
});

/**
 * Registers icons by name, adding to any registered above. Only the icons imported are
 * bundled: Lucide ships each icon as its own ES module.
 *
 * ```ts
 * import { Send, Square } from 'lucide';
 * providers: [provideGlazeIcons({ send: Send, stop: Square })]
 * ```
 */
export function provideGlazeIcons(icons: Readonly<Record<string, GlzIconData>>): Provider {
  return {
    provide: GLZ_ICONS,
    useFactory: (): ReadonlyMap<string, GlzIconData> => {
      const parent = inject(GLZ_ICONS, { skipSelf: true, optional: true });
      return new Map([...(parent ?? new Map<string, GlzIconData>()), ...Object.entries(icons)]);
    },
  };
}

const SVG_NS = 'http://www.w3.org/2000/svg';

const SVG_ATTRIBUTES: Readonly<Record<string, string>> = {
  viewBox: '0 0 24 24',
  width: '100%',
  height: '100%',
  fill: 'none',
  stroke: 'currentColor',
  'stroke-width': '2',
  'stroke-linecap': 'round',
  'stroke-linejoin': 'round',
  focusable: 'false',
};

/**
 * Builds the `<svg>` for `icon` in `document`, element by element: no HTML parsing, so no
 * sanitizing and nothing a Content Security Policy would refuse.
 */
export function iconSvg(document: Document, icon: GlzIconData): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  for (const [key, value] of Object.entries(SVG_ATTRIBUTES)) {
    svg.setAttribute(key, value);
  }
  for (const [tag, attributes] of icon) {
    const child = document.createElementNS(SVG_NS, tag);
    for (const [key, value] of Object.entries(attributes)) {
      if (value !== undefined) {
        child.setAttribute(key, String(value));
      }
    }
    svg.appendChild(child);
  }
  return svg;
}
