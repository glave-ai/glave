import { DeferBlockBehavior, DeferBlockState, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { settle } from '../../testing/links';
import { PersonComponent } from './person.component';

describe('PersonComponent', () => {
  it('shows who is signed in, and opens to Settings and Sign out', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession()),
      ],
    });
    const fixture = TestBed.createComponent(PersonComponent);
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const person = root.querySelector<HTMLButtonElement>('[data-testid="person"]');
    const menu = root.querySelector<HTMLElement>('[data-testid="person-menu"]');
    expect(person?.querySelector('.text')?.textContent.trim()).toBe('You');
    expect(person?.popoverTargetElement).toBe(menu);
    expect(person?.getAttribute('aria-expanded')).toBe('false');
    expect(root.querySelector('[data-testid="nav-settings"]')?.getAttribute('href')).toBe(
      '/settings',
    );
    await expectAccessible(root);
    root.querySelector<HTMLElement>('[data-testid="nav-settings"]')?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    root.querySelector<HTMLButtonElement>('[data-testid="person-sign-out"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(session.signedIn()).toBe(false);
  });

  it('offers Admin only to an admin of an organization the server shows', async () => {
    const shown = { ...SERVER, exists: { organization: true, workers: false }, invites: true };
    for (const [said, offered] of [
      [{ ...shown, role: 'admin' }, true],
      [{ ...shown, role: 'member' }, false],
      [SERVER, false],
    ] as const) {
      TestBed.resetTestingModule();
      TestBed.configureTestingModule({
        providers: [
          provideRouter([{ path: '**', children: [] }]),
          ...provideFakeSession(new FakeSession().answer('server/discover', said)),
        ],
      });
      await TestBed.inject(ServerService).load();
      const fixture = TestBed.createComponent(PersonComponent);
      await fixture.whenStable();
      const admin = (fixture.nativeElement as HTMLElement).querySelector(
        '[data-testid="nav-admin"]',
      );
      expect(admin !== null).toBe(offered);
      if (offered) {
        expect(admin?.getAttribute('href')).toBe('/admin');
      }
    }
  });

  it('offers a person alone who may invite "Invite someone", which opens the invite dialog', async () => {
    const alone = { ...SERVER, origin: 'https://glave.example', invites: true };
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession().answer('server/discover', alone)),
      ],
    });
    await TestBed.inject(ServerService).load();
    const fixture = TestBed.createComponent(PersonComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="nav-admin"]')).toBeNull();
    expect(root.textContent).not.toMatch(/admin|organization/iu);
    root.querySelector<HTMLElement>('[data-testid="nav-invite"]')?.click();
    await settle(fixture);
    expect(document.querySelector<HTMLDialogElement>('[data-testid="invite"]')?.open).toBe(true);
    await expectAccessible(root);
  });

  it('offers no invite where the server takes none, as on a laptop', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession().answer('server/discover', SERVER)),
      ],
    });
    await TestBed.inject(ServerService).load();
    const fixture = TestBed.createComponent(PersonComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="nav-invite"]')).toBeNull();
  });

  it('says so when the invite dialog cannot load its code', async () => {
    TestBed.configureTestingModule({
      deferBlockBehavior: DeferBlockBehavior.Manual,
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(
          new FakeSession().answer('server/discover', {
            ...SERVER,
            origin: 'https://glave.example',
            invites: true,
          }),
        ),
      ],
    });
    await TestBed.inject(ServerService).load();
    const fixture = TestBed.createComponent(PersonComponent);
    await fixture.whenStable();
    const dialog = (await fixture.getDeferBlocks()).at(-1);
    await dialog?.render(DeferBlockState.Error);
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[role="alert"]')?.textContent).toContain('could not load');
    await expectAccessible(root);
  });

  it('switches between the person’s organizations, their own first, the current one checked', async () => {
    const organizations = [
      { id: 'own', people: 1, own: true, role: 'owner', current: true },
      { id: 'acme', people: 12, role: 'member', owner: 'Dana Lee' },
    ];
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession().answer('organization/list', { organizations })),
      ],
    });
    const chosen = vi.spyOn(TestBed.inject(SessionService), 'chooseOrganization').mockReturnValue();
    const fixture = TestBed.createComponent(PersonComponent);
    await settle(fixture);
    const root = fixture.nativeElement as HTMLElement;
    const places = root.querySelectorAll<HTMLElement>('[data-testid="person-switch"]');
    // Read as one line each, past the mark, which only shows.
    const read = (each: HTMLElement): string =>
      each.querySelector('.place')?.textContent.trim() ?? '';
    expect(Array.from(places, read)).toEqual(['Personal Just you', "Dana Lee's team 12 people"]);
    expect(places[0]?.getAttribute('aria-current')).toBe('page');
    await expectAccessible(root);
    places[1]?.click();
    places[0]?.click();
    expect(chosen.mock.calls).toEqual([['acme']]);
  });
});
