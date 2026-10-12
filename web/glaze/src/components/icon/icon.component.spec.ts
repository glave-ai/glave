import { TestBed } from '@angular/core/testing';
import { Send } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { GlzIconComponent } from './icon.component';

describe('GlzIconComponent', () => {
  it('draws the icon given, hidden from screen readers', async () => {
    const fixture = TestBed.createComponent(GlzIconComponent);
    fixture.componentRef.setInput('icon', Send);
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.querySelectorAll('svg path').length).toBe(Send.length);
    expect(host.getAttribute('aria-hidden')).toBe('true');
    expect(host.getAttribute('role')).toBeNull();
  });

  it('is labelled when it means something', async () => {
    const fixture = TestBed.createComponent(GlzIconComponent);
    fixture.componentRef.setInput('icon', Send);
    fixture.componentRef.setInput('label', 'Sent');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.querySelector('svg')).not.toBeNull();
    expect(host.getAttribute('role')).toBe('img');
    expect(host.getAttribute('aria-label')).toBe('Sent');
    await expectAccessible(host);
  });
});
