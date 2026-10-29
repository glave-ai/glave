import { TestBed } from '@angular/core/testing';
import { Router, UrlTree } from '@angular/router';

import { guardAt } from '../testing/links';
import { inviteGuard } from './invite.guard';

describe('inviteGuard', () => {
  const run = (link: string): Promise<unknown> => guardAt(inviteGuard, link);

  it('sends a tab opened with an invite to the page that opens it', async () => {
    const result = await run('/#invite=c0de');
    expect(result).toBeInstanceOf(UrlTree);
    expect(TestBed.inject(Router).serializeUrl(result as UrlTree)).toBe('/invite');
  });

  it('lets any other tab through', async () => {
    expect(await run('/')).toBe(true);
  });
});
