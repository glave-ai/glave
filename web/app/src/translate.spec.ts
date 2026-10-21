import { TestBed } from '@angular/core/testing';
import { clearTranslations } from '@angular/localize';
import { provideRouter } from '@angular/router';
import { GLZ_TEXT, GlzComposerComponent } from '@glave/glaze';

import { PersonComponent } from './app/components/person/person.component';
import { FakeSession, provideFakeSession } from './app/testing/fake-session';
import { translate } from './translate';

describe('translate', () => {
  afterEach(() => {
    clearTranslations();
  });

  it('gives every message read after it in the translation, the app’s and Glaze’s', async () => {
    const providers = await translate('en-XA');
    expect($localize`:@@sideNav.newChat:New chat`).toBe('[Ñéŵ çĥåţ one]');
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession()), ...providers],
    });
    const person = TestBed.createComponent(PersonComponent);
    await person.whenStable();
    const root: HTMLElement = person.nativeElement;
    expect(root.querySelector('[data-testid="person"]')?.textContent).toContain('[Ýöû one]');
    expect(TestBed.inject(GLZ_TEXT).composer.sendLabel).toBe('[Šéñð one]');
    expect(TestBed.createComponent(GlzComposerComponent).componentInstance.sendLabel()).toBe(
      '[Šéñð one]',
    );
  });

  it('writes right to left in ar-XB, keeping what a message is given whole', async () => {
    await translate('ar-XB');
    const name = 'Atlas';
    expect($localize`:@@projectsNav.newIn:New thread in ${name}:PROJECT:`).toBe(
      ['New', 'thread', 'in'].map((word) => `\u200f\u202e${word}\u202c\u200f`).join(' ') + ' Atlas',
    );
  });
});
