import { plural } from './plural';

const FILES = (count: number): string => `{${String(count)}, plural, one {# file} other {# files}}`;

describe('plural', () => {
  it('picks the case English plural rules pick, the number in it', () => {
    expect(plural(FILES(1), 'en')).toBe('1 file');
    expect(plural(FILES(0), 'en')).toBe('0 files');
    expect(plural(FILES(1200), 'en-US')).toBe('1,200 files');
  });

  it('prefers an exact case, and keeps the words around a plural', () => {
    const message = (count: number): string =>
      `Changes, {${String(count)}, plural, =0 {none} one {# file} other {# files}} so far`;
    expect(plural(message(0), 'en')).toBe('Changes, none so far');
    expect(plural(message(3), 'en')).toBe('Changes, 3 files so far');
  });

  it('takes the cases a language has, and other where a translation lacks one', () => {
    const three = new Intl.NumberFormat('ar').format(3);
    const arabic = '{3, plural, one {ملف} few {# ملفات} other {# ملف}}';
    expect(plural(arabic, 'ar')).toBe(`${three} ملفات`);
    expect(plural(FILES(3), 'ar-XB')).toBe(`${three} files`);
  });

  it('writes every plural in a message, and leaves text that is not one alone', () => {
    expect(plural(`${FILES(1)} and ${FILES(2)}`, 'en')).toBe('1 file and 2 files');
    expect(plural('{name} in {braces}', 'en')).toBe('{name} in {braces}');
    expect(plural('{2, plural, one {# file}', 'en')).toBe('{2, plural, one {# file}');
  });
});
