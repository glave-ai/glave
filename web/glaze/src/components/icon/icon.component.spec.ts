import { TestBed } from '@angular/core/testing';
import { Send } from 'lucide';

import { provideGlazeIcons } from '../../tokens/icons';
import { expectAccessible } from '../../testing/axe';
import { GlzIconComponent } from './icon.component';

describe('GlzIconComponent', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideGlazeIcons({ send: Send })] });
  });

  it('draws the icon given, hidden from screen readers', async () => {
    const fixture = TestBed.createComponent(GlzIconComponent);
    fixture.componentRef.setInput('icon', Send);
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.querySelectorAll('svg path').length).toBe(Send.length);
    expect(host.getAttribute('aria-hidden')).toBe('true');
    expect(host.getAttribute('role')).toBeNull();
  });

  it('draws a registered icon by name, labelled when it means something', async () => {
    const fixture = TestBed.createComponent(GlzIconComponent);
    fixture.componentRef.setInput('name', 'send');
    fixture.componentRef.setInput('label', 'Sent');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.querySelector('svg')).not.toBeNull();
    expect(host.getAttribute('role')).toBe('img');
    expect(host.getAttribute('aria-label')).toBe('Sent');
    await expectAccessible(host);
  });

  it('draws nothing for a name not registered', async () => {
    const fixture = TestBed.createComponent(GlzIconComponent);
    fixture.componentRef.setInput('name', 'nothing');
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).querySelector('svg')).toBeNull();
  });
});
