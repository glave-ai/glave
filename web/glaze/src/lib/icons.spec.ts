import { Send } from 'lucide';

import { iconSvg } from './icons';

describe('icons', () => {
  it('draws an icon as SVG elements, with the stroke style', () => {
    const svg = iconSvg(document, Send);
    expect(svg.namespaceURI).toBe('http://www.w3.org/2000/svg');
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24');
    expect(svg.getAttribute('stroke')).toBe('currentColor');
    expect(svg.children.length).toBe(Send.length);
    expect(svg.querySelector('path')?.getAttribute('d')).toBe(Send[0]?.[1]['d']);
  });
});
