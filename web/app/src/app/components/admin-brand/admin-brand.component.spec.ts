import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ChangesService } from '../../services/changes.service';
import { FakeSession, FakeTurn, provideFakeSession } from '../../testing/fake-session';
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

function typeName(root: HTMLElement, name: string): void {
  const input = byId(root, 'brand-name') as HTMLInputElement;
  input.value = name;
  input.dispatchEvent(new Event('input'));
}

const TEAL = { color: { color: '#0f766e', adjust: true } };

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

  describe('when another admin changes the brand', () => {
    const THEIRS = { name: 'Theirs', logo: 'def', accent: TEAL, locked: true };
    const heard = (): void => {
      TestBed.inject(ChangesService).tell('brand');
    };
    const withLogo = (fake: FakeSession, logo: string): FakeSession =>
      fake
        .answer('appearance/get', {
          organization: { name: 'Acme', logo, accent: 'plain', locked: false },
        })
        .answer('brand/logo/get', {
          logo: { sha256: logo, media_type: 'image/png', data: 'eA==' },
        });
    const kept = (): FakeSession =>
      new FakeSession()
        .answer('brand/get', {
          brand: { name: 'Acme', logo: 'abc', accent: 'plain', locked: false },
        })
        .answer('brand/set', (params) => ({ brand: params }));

    it('shows their change while nothing here is edited, and Save keeps it', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      fake.answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Theirs');
      expect(byId(root, 'brand-locked-line')?.textContent).toContain('Nobody');
      expect(byId(root, 'brand-preview-light')).not.toBeNull();
      await expectAccessible(root);
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([THEIRS]);
    });

    it('hears it on a page opened right here, with no chats open before it', async () => {
      const watch = new FakeTurn();
      const fake = kept().listWatch(watch);
      const { root, settle } = await render(fake);
      await vi.waitFor(() => {
        expect(fake.paramsOf('thread/list/watch')).toHaveLength(1);
      });
      fake.answer('brand/get', { brand: THEIRS });
      watch.send({ jsonrpc: '2.0', method: 'settings/changed', params: { setting: 'brand' } });
      await settle();
      await vi.waitFor(() => {
        expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Theirs');
      });
    });

    it('keeps what is edited here, though they changed it too, and takes the rest from them', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      typeName(root, 'Mine');
      fake.answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
      expect(byId(root, 'brand-locked-line')?.textContent).toContain('Nobody');
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([{ ...THEIRS, name: 'Mine' }]);
    });

    it('keeps a logo removed here, when they kept another', async () => {
      const fake = withLogo(kept(), 'abc');
      const { root, settle } = await render(fake);
      await vi.waitFor(() => {
        expect(byId(root, 'brand-logo-remove')).not.toBeNull();
      });
      byId(root, 'brand-logo-remove')?.click();
      await settle();
      expect(byId(root, 'brand-logo')).toBeNull();
      fake.answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([{ name: 'Theirs', accent: TEAL, locked: true }]);
    });

    it('draws their logo, not a file picked here and saved, once they keep another', async () => {
      const fake = withLogo(kept(), 'abc').answer('brand/logo/upload', { sha256: 'ghi' });
      const { root, settle } = await render(fake);
      const file = byId(root, 'brand-logo-file') as HTMLInputElement;
      Object.defineProperty(file, 'files', { value: [new File(['x'], 'logo.png')] });
      file.dispatchEvent(new Event('change'));
      await vi.waitFor(() => {
        expect(fake.paramsOf('brand/logo/upload')).toHaveLength(1);
      });
      await settle();
      await vi.waitFor(() => {
        expect((byId(root, 'brand-save') as HTMLButtonElement).disabled).toBe(false);
      });
      const picked = (byId(root, 'brand-logo') as HTMLImageElement).src;
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([
        { name: 'Acme', logo: 'ghi', accent: 'plain', locked: false },
      ]);
      expect((byId(root, 'brand-logo') as HTMLImageElement).src).toBe(picked);
      fake.answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      await vi.waitFor(() => {
        expect((byId(root, 'brand-logo') as HTMLImageElement).src).not.toBe(picked);
      });
    });

    it('reads again before Save, so a change it never heard is not overwritten', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      typeName(root, 'Mine');
      fake.answer('brand/get', { brand: THEIRS });
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([{ ...THEIRS, name: 'Mine' }]);
      expect(byId(root, 'brand-locked-line')?.textContent).toContain('Nobody');
    });

    it('keeps nothing, and what is edited, when it cannot read before Save', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      typeName(root, 'Mine');
      fake.answer(
        'brand/get',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
      );
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([]);
      expect(byId(root, 'brand-problem')?.textContent).toContain('not answering');
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
    });

    it('takes what was saved as read: hearing its own save changes nothing', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      typeName(root, 'Mine');
      byId(root, 'brand-save')?.click();
      await settle();
      expect(byId(root, 'brand-saved')?.textContent).toContain('Saved');
      fake.answer('brand/get', {
        brand: { name: 'Mine', logo: 'abc', accent: 'plain', locked: false },
      });
      heard();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
      expect(byId(root, 'brand-saved')?.textContent).toContain('Saved');
      fake.answer('brand/get', { brand: { ...THEIRS, name: 'Theirs' } });
      heard();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Theirs');
    });

    it('does not let a read answered after a save take back what it saved', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      const answerRead = fake.held('brand/get');
      heard();
      await settle();
      typeName(root, 'Mine');
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([]);
      // The read began before the save, and the save waits for it, then reads for itself.
      const next = {
        brand: { name: 'Acme', logo: 'abc', accent: 'plain', locked: false },
      };
      answerRead(next);
      await settle();
      answerRead(next);
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([
        { name: 'Mine', logo: 'abc', accent: 'plain', locked: false },
      ]);
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
    });

    it('offers Save once a read that failed comes back, on hearing a change', async () => {
      const fake = new FakeSession().answer(
        'brand/get',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
      );
      const { root, settle } = await render(fake);
      fake.answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      expect((byId(root, 'brand-save') as HTMLButtonElement).disabled).toBe(false);
      expect(byId(root, 'brand-problem')).toBeNull();
    });
  });
});
