import { Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Send, X } from 'lucide';

import { GLZ_ICONS, iconSvg, provideGlazeIcons } from './icons';

describe('icons', () => {
  it('draws an icon as SVG elements, with the stroke style', () => {
    const svg = iconSvg(document, Send);
    expect(svg.namespaceURI).toBe('http://www.w3.org/2000/svg');
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24');
    expect(svg.getAttribute('stroke')).toBe('currentColor');
    expect(svg.children.length).toBe(Send.length);
    expect(svg.querySelector('path')?.getAttribute('d')).toBe(Send[0]?.[1]['d']);
  });

  it('registers icons by name, adding to those registered above', () => {
    const parent = Injector.create({ providers: [provideGlazeIcons({ send: Send })] });
    const child = Injector.create({ providers: [provideGlazeIcons({ x: X })], parent });
    const icons = child.get(GLZ_ICONS);
    expect(icons.get('send')).toBe(Send);
    expect(icons.get('x')).toBe(X);
    expect(TestBed.inject(GLZ_ICONS).size).toBe(0);
  });
});
