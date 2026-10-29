import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { AdminComponent } from './admin.component';

describe('AdminComponent', () => {
  it('leads back to the chats and to People', async () => {
    TestBed.configureTestingModule({
      providers: [...provideFakeSession(new FakeSession()), provideRouter([])],
    });
    const fixture = TestBed.createComponent(AdminComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="admin-back"]')?.getAttribute('href')).toBe('/');
    const people = root.querySelector('[data-testid="admin-people"]');
    expect(people?.getAttribute('href')).toBe('/admin');
    expect(people?.getAttribute('aria-current')).toBe('page');
    await expectAccessible(root);
  });
});
