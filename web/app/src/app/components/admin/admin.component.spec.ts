import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { AdminComponent } from './admin.component';

describe('AdminComponent', () => {
  it('leads back to the chats, to People and to Brand, marking the one open', async () => {
    TestBed.configureTestingModule({
      providers: [
        ...provideFakeSession(new FakeSession()),
        provideRouter([
          { path: 'admin', children: [] },
          { path: 'admin/brand', children: [] },
        ]),
      ],
    });
    const fixture = TestBed.createComponent(AdminComponent);
    await TestBed.inject(Router).navigateByUrl('/admin');
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="admin-back"]')?.getAttribute('href')).toBe('/');
    const people = root.querySelector('[data-testid="admin-people"]');
    const brand = root.querySelector('[data-testid="admin-brand"]');
    expect(people?.getAttribute('href')).toBe('/admin');
    expect(people?.getAttribute('aria-current')).toBe('page');
    expect(brand?.getAttribute('aria-current')).toBeNull();
    await expectAccessible(root);

    await TestBed.inject(Router).navigateByUrl('/admin/brand');
    await fixture.whenStable();
    expect(people?.getAttribute('aria-current')).toBeNull();
    expect(brand?.getAttribute('aria-current')).toBe('page');
  });
});
