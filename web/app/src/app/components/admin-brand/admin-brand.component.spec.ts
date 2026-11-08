import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ChangesService } from '../../services/changes.service';
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
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, settle: () => settle(fixture) };
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
const ACME = { name: 'Acme', logo: 'abc', accent: 'plain', locked: false };

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
    expect(byId(root, 'brand-problem')?.textContent).toContain('PNG, JPEG or WebP');
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
      expect(byId(root, 'brand-logo-remove')).not.toBeNull();
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
      const first = (byId(root, 'brand-logo') as HTMLImageElement).src;
      const file = byId(root, 'brand-logo-file') as HTMLInputElement;
      Object.defineProperty(file, 'files', { value: [new File(['x'], 'logo.png')] });
      file.dispatchEvent(new Event('change'));
      await settle();
      const picked = (byId(root, 'brand-logo') as HTMLImageElement).src;
      expect(picked).not.toBe(first);
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([{ ...ACME, logo: 'ghi' }]);
      expect((byId(root, 'brand-logo') as HTMLImageElement).src).toBe(picked);
      withLogo(fake, 'def').answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      expect(fake.paramsOf('brand/logo/get')).toHaveLength(2);
      const shown = (byId(root, 'brand-logo') as HTMLImageElement).src;
      expect([first, picked]).not.toContain(shown);
    });

    it('keeps a color and how it is tuned together, when either is edited here', async () => {
      const fake = kept().answer('brand/get', { brand: { ...ACME, accent: TEAL } });
      const { root, settle } = await render(fake);
      byId(root, 'brand-adjust-input')?.click();
      await settle();
      fake.answer('brand/get', {
        brand: { ...ACME, accent: { color: { color: '#b4235a', adjust: true } } },
      });
      heard();
      await settle();
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([
        { ...ACME, accent: { color: { color: '#0f766e', adjust: false } } },
      ]);
    });

    it('says nothing when a read on hearing a change fails, and Save reads again', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      fake.answer(
        'brand/get',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
      );
      heard();
      await settle();
      expect(byId(root, 'brand-problem')).toBeNull();
      fake.answer('brand/get', { brand: THEIRS });
      byId(root, 'brand-save')?.click();
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([THEIRS]);
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
      fake.answer('brand/get', { brand: { ...ACME, name: 'Mine' } });
      heard();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
      expect(byId(root, 'brand-saved')?.textContent).toContain('Saved');
      fake.answer('brand/get', { brand: THEIRS });
      heard();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Theirs');
    });

    it('shows the name as the server kept it, trimmed', async () => {
      const fake = kept().answer('brand/set', { brand: { ...ACME, name: 'Mine' } });
      const { root, settle } = await render(fake);
      typeName(root, 'Mine ');
      byId(root, 'brand-save')?.click();
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
      expect(byId(root, 'brand-saved')?.textContent).toContain('Saved');
    });

    it('does not say Saved over what was typed while the save was on its way', async () => {
      const fake = kept();
      const answerSave = fake.held('brand/set');
      const { root, settle } = await render(fake);
      byId(root, 'brand-save')?.click();
      await settle();
      typeName(root, 'Mine');
      answerSave({ brand: ACME });
      await settle();
      expect((byId(root, 'brand-name') as HTMLInputElement).value).toBe('Mine');
      expect(byId(root, 'brand-saved')?.textContent).not.toContain('Saved');
    });

    it('sends reads and saves one at a time, so an older answer can’t undo a newer', async () => {
      const fake = kept();
      const { root, settle } = await render(fake);
      const reads: ((answer: object) => void)[] = [];
      fake.answer(
        'brand/get',
        () =>
          new Promise((resolve) => {
            reads.push(resolve);
          }),
      );
      const answerSave = fake.held('brand/set');
      heard();
      await settle();
      typeName(root, 'Mine');
      byId(root, 'brand-save')?.click();
      await settle();
      // The save waits for the read begun before it, which says something else is kept.
      expect(reads).toHaveLength(1);
      reads[0]?.({ brand: { ...ACME, locked: true } });
      await settle();
      expect(reads).toHaveLength(2);
      expect(fake.paramsOf('brand/set')).toEqual([]);
      reads[1]?.({ brand: { ...ACME, locked: true } });
      await settle();
      expect(fake.paramsOf('brand/set')).toEqual([{ ...ACME, name: 'Mine', locked: true }]);
      // A read heard now waits for the save's answer.
      heard();
      await settle();
      expect(reads).toHaveLength(2);
      answerSave({ brand: { ...ACME, name: 'Mine', locked: true } });
      await settle();
      expect(reads).toHaveLength(3);
      reads[2]?.({ brand: { ...ACME, name: 'Mine', locked: true } });
      await settle();
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
