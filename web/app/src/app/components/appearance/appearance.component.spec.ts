import { TestBed } from '@angular/core/testing';
import type { Accent } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ThemeService } from '../../services/theme.service';
import { AppearanceComponent } from './appearance.component';

const SHADES = { light: '#0f766e', dark: '#59c5b8' };

/** The server's answer for `accent`. */
function answered(accent: Accent): object {
  return accent === 'plain' ? { chosen: 'plain' } : { chosen: accent, shades: SHADES };
}

async function render(): Promise<{
  root: HTMLElement;
  fake: FakeSession;
  settle: () => Promise<void>;
}> {
  const fake = new FakeSession().answer('appearance/set', (params) =>
    answered((params as { readonly accent: Accent }).accent),
  );
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(AppearanceComponent);
  const settle = async (): Promise<void> => {
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
  };
  await settle();
  return { root: fixture.nativeElement as HTMLElement, fake, settle };
}

function text(root: HTMLElement, id: string): string {
  return root.querySelector(`[data-testid="${id}"]`)?.textContent.trim() ?? '';
}

describe('AppearanceComponent', () => {
  it('starts plain and tuned, with no word of an organization', async () => {
    const { root } = await render();
    expect(text(root, 'appearance-line')).toBe('You’re using glave’s plain look.');
    expect(text(root, 'appearance-adjust-line')).toContain('tunes your color');
    expect(root.textContent).not.toMatch(/organization|admin|policy/iu);
    await expectAccessible(root);
  });

  it('chooses a swatch, and turning tuning off asks for the exact color, saying what it risks', async () => {
    const { root, fake, settle } = await render();
    root.querySelector<HTMLInputElement>('input[aria-label="Teal"]')?.click();
    await settle();
    expect(fake.paramsOf('appearance/set').at(-1)).toEqual({
      accent: { color: { color: '#0f766e', adjust: true } },
    });
    expect(text(root, 'appearance-line')).toBe('You’re using your own color.');

    root.querySelector<HTMLInputElement>('[data-testid="appearance-adjust-input"]')?.click();
    await settle();
    expect(fake.paramsOf('appearance/set').at(-1)).toEqual({
      accent: { color: { color: '#0f766e', adjust: false } },
    });
    expect(text(root, 'appearance-adjust-line')).toBe(
      'Your exact color, everywhere. Some text may be hard to read.',
    );
  });

  it('keeps light or dark in this browser', async () => {
    const { root, fake, settle } = await render();
    const dark = Array.from(
      root.querySelectorAll<HTMLElement>('[data-testid="settings-theme-option"]'),
    ).find((option) => option.textContent.includes('Dark'));
    dark?.querySelector('input')?.click();
    await settle();
    expect(TestBed.inject(ThemeService).choice()).toBe('dark');
    expect(fake.paramsOf('appearance/set')).toEqual([]);
    TestBed.inject(ThemeService).set('system');
  });
});
