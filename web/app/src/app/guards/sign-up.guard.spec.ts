import { TestBed } from '@angular/core/testing';
import { Router, UrlTree } from '@angular/router';

import { guardAt } from '../testing/links';
import { signUpGuard } from './sign-up.guard';

describe('signUpGuard', () => {
  const run = (link: string): Promise<unknown> => guardAt(signUpGuard, link);

  it('sends a tab opened with a sign-up link to the page that makes its passkey', async () => {
    const result = await run('/#signup=c0de');
    expect(result).toBeInstanceOf(UrlTree);
    expect(TestBed.inject(Router).serializeUrl(result as UrlTree)).toBe('/sign-up');
  });

  it('lets any other tab through', async () => {
    expect(await run('/')).toBe(true);
  });
});
