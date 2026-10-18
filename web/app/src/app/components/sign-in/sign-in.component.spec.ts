import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SignInComponent } from './sign-in.component';

describe('SignInComponent', () => {
  it('says how to sign in, in plain words', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession(), null)],
    });
    const fixture = TestBed.createComponent(SignInComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('header glz-top-bar')?.textContent).toContain('glave');
    const said = root.querySelector('main')?.textContent ?? '';
    expect(said).toContain('Open glave on this computer again');
    expect(said).not.toMatch(/serve|--|config|terminal/u);
    await expectAccessible(root);
  });

  it('goes to the workspace once signed in', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession())],
    });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(SignInComponent);
    await fixture.whenStable();
    expect(navigate).toHaveBeenCalledWith('/');
  });
});
