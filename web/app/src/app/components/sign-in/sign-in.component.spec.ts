import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SignInComponent } from './sign-in.component';

describe('SignInComponent', () => {
  it('says how to sign in', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession(), false)],
    });
    const fixture = TestBed.createComponent(SignInComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('header glz-top-bar')?.textContent).toContain('glave');
    expect(root.querySelector('main')?.textContent).toContain('glave serve --http');
    await expectAccessible(root);
  });

  it('goes to the workspace once signed in', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession(), true)],
    });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(SignInComponent);
    await fixture.whenStable();
    expect(navigate).toHaveBeenCalledWith('/');
  });
});
