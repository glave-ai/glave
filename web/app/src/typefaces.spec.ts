import { typefaces } from './typefaces';

describe('typefaces', () => {
  it('loads each typeface the app draws in, every weight it uses', async () => {
    const asked: string[] = [];
    await typefaces({
      load: (font) => {
        asked.push(font);
        return Promise.resolve([]);
      },
    });
    expect(asked).toEqual([
      '300 1em "Atkinson Hyperlegible Next"',
      '400 1em "Atkinson Hyperlegible Next"',
      '700 1em "Atkinson Hyperlegible Next"',
      '400 1em "Atkinson Hyperlegible Mono"',
    ]);
  });

  it('goes on when one fails, or once its limit has passed', async () => {
    await expect(
      typefaces({ load: () => Promise.reject(new Error('the font did not load')) }),
    ).resolves.toBeUndefined();
    await expect(typefaces({ load: () => new Promise(() => undefined) }, 1)).resolves.toBe(
      undefined,
    );
  });
});
