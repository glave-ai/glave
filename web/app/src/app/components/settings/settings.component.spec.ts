import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { SessionService } from '../../services/session.service';
import { ThemeService } from '../../services/theme.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MODELS } from '../../testing/fixtures';

const SERVER = {
  name: 'glave',
  version: '0.1.0',
  protocols: ['2026-10'],
  approval: 'auto',
  approvals: ['ask', 'auto'],
} as const;
import { SettingsComponent } from './settings.component';

describe('SettingsComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the server, picks a theme and signs out', async () => {
    const fake = new FakeSession()
      .answer('server/discover', SERVER)
      .answer('provider/list', { providers: [] })
      .answer('credential/list', { credentials: [], device: false });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(SettingsComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="settings-server"]')?.textContent).toContain(
      'glave 0.1.0 · protocol 2026-10 · approvals: auto',
    );
    expect(root.textContent).not.toMatch(/config|policy|toml|--/u);
    expect(root.querySelector('[data-testid="settings-no-accounts"]')).not.toBeNull();
    // Memory lives in Settings, on its own page.
    expect(root.querySelector('[data-testid="settings-memory"]')?.getAttribute('href')).toBe(
      '/memory',
    );
    const themes = Array.from(root.querySelectorAll('glz-radio'), (radio) =>
      radio.textContent.trim(),
    );
    expect(themes).toEqual(['System', 'Light', 'Dark']);
    await expectAccessible(root);

    root.querySelectorAll<HTMLInputElement>('input[type="radio"]')[2]?.click();
    expect(TestBed.inject(ThemeService).choice()).toBe('dark');

    root.querySelector<HTMLButtonElement>('[data-testid="settings-sign-out-others"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(fake.paramsOf('signin/end_others')).toEqual([{}]);
    expect(root.querySelector('[data-testid="settings-signed-out-others"]')?.textContent).toContain(
      'This tab stays signed in.',
    );
    expect(TestBed.inject(SessionService).signedIn()).toBe(true);
    await expectAccessible(root);

    root.querySelector<HTMLButtonElement>('[data-testid="settings-sign-out"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(fake.paramsOf('signin/end')).toEqual([{}]);
    expect(TestBed.inject(SessionService).signedIn()).toBe(false);
    expect(navigate).toHaveBeenCalledWith('/sign-in');
  });

  it('lists the model accounts by service and key ending, and removes one once confirmed', async () => {
    const account = {
      id: '01900000-0000-7000-8000-000000000301',
      kind: 'openai',
      hint: 'a1b2',
      models: ['gpt-6-luna'],
    } as const;
    let kept = true;
    const fake = new FakeSession()
      .answer('server/discover', SERVER)
      .answer('provider/list', () => ({ providers: kept ? [account] : [] }))
      .answer('credential/list', { credentials: [], device: false })
      .answer('provider/remove', () => {
        kept = false;
        return {};
      })
      .answer('model/list', () => ({ models: kept ? MODELS : [] }));
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(SettingsComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const row = root.querySelector('[data-testid="settings-account"]');
    expect(row?.textContent).toContain('OpenAI');
    expect(row?.textContent).toContain('Key ending a1b2');
    expect(
      root.querySelector('[data-testid="settings-account-change"]')?.getAttribute('href'),
    ).toBe('/connect?kind=openai');
    await expectAccessible(root);

    root.querySelector<HTMLButtonElement>('[data-testid="settings-account-remove"]')?.click();
    await fixture.whenStable();
    document.querySelector<HTMLButtonElement>('[data-testid="settings-remove-ok"]')?.click();
    for (let turn = 0; turn < 5; turn += 1) {
      await new Promise((resolve) => setTimeout(resolve));
    }
    await fixture.whenStable();
    expect(fake.paramsOf('provider/remove')).toEqual([{ provider: account.id }]);
    expect(root.querySelector('[data-testid="settings-account"]')).toBeNull();
    expect(navigate).toHaveBeenCalledWith('/connect');
  });
});
