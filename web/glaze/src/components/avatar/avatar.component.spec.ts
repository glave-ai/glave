import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzAvatarComponent } from './avatar.component';

describe('GlzAvatarComponent', () => {
  it('shows initials, named for screen readers', async () => {
    const fixture = TestBed.createComponent(GlzAvatarComponent);
    fixture.componentRef.setInput('name', 'Ada Lovelace');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.getAttribute('role')).toBe('img');
    expect(host.getAttribute('aria-label')).toBe('Ada Lovelace');
    expect(host.textContent.trim()).toBe('AL');
    expect(host.querySelector('[data-testid="avatar-hope"]')).toBeNull();
    await expectAccessible(host);
  });

  it('shows a picture, and Hope presence when asked', async () => {
    const fixture = TestBed.createComponent(GlzAvatarComponent);
    fixture.componentRef.setInput('name', 'Hope');
    fixture.componentRef.setInput('src', '/hope.png');
    fixture.componentRef.setInput('hope', true);
    fixture.componentRef.setInput('size', 'lg');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.querySelector('img')?.getAttribute('src')).toBe('/hope.png');
    expect(host.querySelector('[data-testid="avatar-hope"]')).not.toBeNull();
    expect(host.classList).toContain('glz-avatar-lg');
  });
});
