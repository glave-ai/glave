import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { AdminBrandComponent } from './admin-brand.component';

const SHADES = { light: '#0f766e', dark: '#59c5b8' };

async function render(fake: FakeSession): Promise<{
  root: HTMLElement;
  settle: () => Promise<void>;
}> {
  fake.answer('appearance/preview', { shades: SHADES });
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(AdminBrandComponent);
  const settle = async (): Promise<void> => {
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
  };
  await settle();
  return { root: fixture.nativeElement as HTMLElement, settle };
}

function byId(root: HTMLElement, id: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${id}"]`);
}

describe('AdminBrandComponent', () => {
  it('shows what is kept, and the server’s values for it in light and dark', async () => {
    const { root } = await render(
      new FakeSession().answer('brand/get', {
        brand: {
          name: 'Acme',
          accent: { color: { color: '#0f766e', adjust: true } },
          locked: true,
        },
      }),
    );
    expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Acme');
    expect(byId(root, 'brand-preview-light')?.style.getPropertyValue('--shade')).toBe(SHADES.light);
    expect(byId(root, 'brand-preview-dark')?.style.getPropertyValue('--shade')).toBe(SHADES.dark);
    expect(byId(root, 'brand-locked-line')?.textContent).toContain('Nobody');
    await expectAccessible(root);
  });

  it('keeps nothing until Save, then all of it, and says so', async () => {
    const fake = new FakeSession().answer('brand/set', {
      brand: { accent: 'plain', locked: false },
    });
    const { root, settle } = await render(fake);
    const name = byId(root, 'brand-name') as HTMLInputElement;
    name.value = 'Acme';
    name.dispatchEvent(new Event('input'));
    root.querySelector<HTMLInputElement>('input[aria-label="Teal"]')?.click();
    byId(root, 'brand-locked-input')?.click();
    await settle();
    expect(fake.paramsOf('brand/set')).toEqual([]);
    byId(root, 'brand-save')?.click();
    await settle();
    expect(fake.paramsOf('brand/set')).toEqual([
      {
        name: 'Acme',
        accent: { color: { color: '#0f766e', adjust: true } },
        locked: true,
      },
    ]);
    expect(byId(root, 'brand-saved')?.textContent).toContain('Saved');
  });

  it('says why a logo is refused, and keeps what it has', async () => {
    const fake = new FakeSession().answer(
      'brand/logo/upload',
      new ApiError({
        code: ErrorCode.INVALID_PARAMS,
        message: 'a logo has to be a PNG, JPEG or WebP picture',
      }),
    );
    const { root, settle } = await render(fake);
    const file = byId(root, 'brand-logo-file') as HTMLInputElement;
    Object.defineProperty(file, 'files', { value: [new File(['x'], 'logo.gif')] });
    file.dispatchEvent(new Event('change'));
    await settle();
    await vi.waitFor(() => {
      expect(byId(root, 'brand-problem')?.textContent).toContain('PNG, JPEG or WebP');
    });
    expect(byId(root, 'brand-logo')).toBeNull();
  });

  it('offers no Save until it has read what is kept, so none overwrites it unread', async () => {
    const { root } = await render(
      new FakeSession().answer(
        'brand/get',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
      ),
    );
    expect((byId(root, 'brand-save') as HTMLButtonElement).disabled).toBe(true);
    expect(byId(root, 'brand-problem')?.textContent).toContain('not answering');
  });

  it('keeps the logo it has when saving, and none once it is removed', async () => {
    const fake = new FakeSession()
      .answer('brand/get', { brand: { logo: 'abc', accent: 'plain', locked: false } })
      .answer('brand/set', { brand: { accent: 'plain', locked: false } });
    const { root, settle } = await render(fake);
    byId(root, 'brand-save')?.click();
    await settle();
    expect(fake.paramsOf('brand/set')).toEqual([{ logo: 'abc', accent: 'plain', locked: false }]);
  });
});
