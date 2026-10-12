import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { SessionService } from '../../services/session.service';
import { ThemeService } from '../../services/theme.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SettingsComponent } from './settings.component';

describe('SettingsComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the server, picks a theme and signs out', async () => {
    const fake = new FakeSession().answer('server/discover', {
      name: 'glave',
      version: '0.1.0',
      protocols: ['2026-10'],
      approval: 'auto',
    });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(SettingsComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="settings-server"]')?.textContent).toContain(
      'glave 0.1.0 · protocol 2026-10 · approvals: auto',
    );
    expect(root.textContent).toContain('glave policy explain');
    await expectAccessible(root);

    root.querySelectorAll<HTMLInputElement>('input[type="radio"]')[2]?.click();
    expect(TestBed.inject(ThemeService).choice()).toBe('dark');

    root.querySelector<HTMLButtonElement>('[data-testid="settings-sign-out"]')?.click();
    await fixture.whenStable();
    expect(TestBed.inject(SessionService).signedIn()).toBe(false);
    expect(navigate).toHaveBeenCalledWith('/sign-in');
  });
});
