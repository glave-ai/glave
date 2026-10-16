import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzAvatarComponent } from './avatar.component';

describe('GlzAvatarComponent', () => {
  it('shows initials, named for screen readers', async () => {
    const fixture = TestBed.createComponent(GlzAvatarComponent);
    fixture.componentRef.setInput('testId', 'avatar');
    fixture.componentRef.setInput('name', 'Ada Lovelace');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.getAttribute('role')).toBe('img');
    expect(host.getAttribute('aria-label')).toBe('Ada Lovelace');
    expect(host.textContent.trim()).toBe('AL');
    await expectAccessible(host);
  });

  it('shows a picture when it has one', async () => {
    const fixture = TestBed.createComponent(GlzAvatarComponent);
    fixture.componentRef.setInput('testId', 'avatar');
    fixture.componentRef.setInput('name', 'Ada Lovelace');
    fixture.componentRef.setInput('src', '/ada.png');
    fixture.componentRef.setInput('size', 'lg');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.querySelector('img')?.getAttribute('src')).toBe('/ada.png');
    expect(host.classList).toContain('glz-avatar-lg');
  });
});
