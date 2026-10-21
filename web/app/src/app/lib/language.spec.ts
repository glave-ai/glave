import { chooseLanguage } from './language';

describe('language', () => {
  it('reads English in any region, its dates and numbers in that region', () => {
    expect(chooseLanguage(['en-GB', 'en'])).toEqual({
      messages: 'en',
      locale: 'en-GB',
      direction: 'ltr',
    });
    expect(chooseLanguage(['en'])).toEqual({ messages: 'en', locale: 'en', direction: 'ltr' });
  });

  it('takes the first language the app has, in the order the browser prefers them', () => {
    expect(chooseLanguage(['fr-FR', 'fr', 'en-US']).locale).toBe('en-US');
    expect(chooseLanguage(['ar-XB', 'en-US']).messages).toBe('ar-XB');
  });

  it('falls back to English in the region the browser names first', () => {
    expect(chooseLanguage(['de-DE', 'de'])).toEqual({
      messages: 'en',
      locale: 'en-DE',
      direction: 'ltr',
    });
    expect(chooseLanguage(['de']).locale).toBe('en');
    expect(chooseLanguage([]).locale).toBe('en');
  });

  it('gives a pseudo-locale only to a browser that names it exactly, in any case', () => {
    expect(chooseLanguage(['en-xa'])).toEqual({
      messages: 'en-XA',
      locale: 'en-XA',
      direction: 'ltr',
    });
    expect(chooseLanguage(['ar-EG', 'ar'])).toEqual({
      messages: 'en',
      locale: 'en-EG',
      direction: 'ltr',
    });
  });

  it('writes a right-to-left language right to left', () => {
    expect(chooseLanguage(['ar-XB'])).toEqual({
      messages: 'ar-XB',
      locale: 'ar-XB',
      direction: 'rtl',
    });
  });

  it('passes over what is not a language tag', () => {
    expect(chooseLanguage(['en-', 'not a tag', 'en-AU']).locale).toBe('en-AU');
  });
});
