import { Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Send, X } from 'lucide';

import { GLZ_ICONS, provideGlazeIcons } from './icons';

describe('icon tokens', () => {
  it('registers icons by name, adding to those registered above', () => {
    const parent = Injector.create({ providers: [provideGlazeIcons({ send: Send })] });
    const child = Injector.create({ providers: [provideGlazeIcons({ x: X })], parent });
    const icons = child.get(GLZ_ICONS);
    expect(icons.get('send')).toBe(Send);
    expect(icons.get('x')).toBe(X);
    expect(TestBed.inject(GLZ_ICONS).size).toBe(0);
  });
});
