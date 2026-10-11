import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import {
  type ActivatedRouteSnapshot,
  Router,
  type RouterStateSnapshot,
  UrlTree,
} from '@angular/router';

import { SessionService } from '../services/session.service';
import { signedInGuard } from './signed-in.guard';

describe('signedInGuard', () => {
  function run(signedIn: boolean): ReturnType<typeof signedInGuard> {
    TestBed.configureTestingModule({
      providers: [{ provide: SessionService, useValue: { signedIn: signal(signedIn) } }],
    });
    return TestBed.runInInjectionContext(() =>
      signedInGuard({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
    );
  }

  it('lets a signed-in person in', () => {
    expect(run(true)).toBe(true);
  });

  it('sends anyone else to the sign-in page', () => {
    const result = run(false);
    expect(result).toBeInstanceOf(UrlTree);
    expect(TestBed.inject(Router).serializeUrl(result as UrlTree)).toBe('/sign-in');
  });
});
