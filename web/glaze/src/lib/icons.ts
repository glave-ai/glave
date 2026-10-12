import type { IconNode } from 'lucide';

/** An icon's drawing in Lucide's format: SVG child elements and their attributes. */
export type GlzIconData = IconNode;

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
