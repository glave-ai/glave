import { ApplicationInitStatus } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { appConfig } from './app.config';
import { FakeSession, provideFakeSession } from './testing/fake-session';

describe('appConfig', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('applies the theme the person picked as the app starts, before Settings is opened', async () => {
    localStorage.setItem('glave-theme', 'light');
    TestBed.configureTestingModule({
      providers: [...appConfig.providers, ...provideFakeSession(new FakeSession())],
    });
    await TestBed.inject(ApplicationInitStatus).donePromise;
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
  });
});
