import { chooseLanguage } from './language';

describe('language', () => {
  it('reads English in any region, its dates and numbers in that region', () => {
    expect(chooseLanguage(['en-GB', 'en'])).toEqual({ messages: 'en', locale: 'en-GB' });
    expect(chooseLanguage(['en'])).toEqual({ messages: 'en', locale: 'en' });
  });

  it('takes the first language the app has, in the order the browser prefers them', () => {
    expect(chooseLanguage(['fr-FR', 'fr', 'en-US'])).toEqual({ messages: 'en', locale: 'en-US' });
    expect(chooseLanguage(['ar-XB', 'en-US'])).toEqual({ messages: 'ar-XB', locale: 'ar-XB' });
  });

  it('falls back to English when the app has none of them', () => {
    expect(chooseLanguage(['fr-FR', 'de'])).toEqual({ messages: 'en', locale: 'en' });
    expect(chooseLanguage([])).toEqual({ messages: 'en', locale: 'en' });
  });

  it('gives a pseudo-locale only to a browser that names it exactly, in any case', () => {
    expect(chooseLanguage(['en-xa'])).toEqual({ messages: 'en-XA', locale: 'en-xa' });
    expect(chooseLanguage(['ar-EG', 'ar'])).toEqual({ messages: 'en', locale: 'en' });
  });
});
