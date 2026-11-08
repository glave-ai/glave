import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { TOKEN_KEY } from '../lib/token';
import { FAKE_SERVER, FakeSession, SIGNED_IN, provideFakeSession } from '../testing/fake-session';
import { OrganizationsService } from './organizations.service';
import { SessionService } from './session.service';
import { TapsService } from './taps.service';

const THREAD = '019a0000-0000-7000-8000-000000000001';
const HERE = '019a0000-0000-7000-8000-0000000000aa';
const ELSEWHERE = '019a0000-0000-7000-8000-0000000000bb';

/** Asks the app to open `page` in `org`, as the worker does: whether it said it did. */
function ask(taps: TapsService, page: string, org: string | null): boolean {
  const answers: unknown[] = [];
  const port = { postMessage: (answer: unknown) => answers.push(answer) } as unknown as MessagePort;
  taps.heard({ data: { glave: 'open', page, org }, ports: [port] });
  return answers.length > 0;
}

describe('TapsService', () => {
  beforeEach(async () => {
    const fake = new FakeSession()
      .answer('server/discover', { id: FAKE_SERVER })
      .answer('organization/list', {
        organizations: [{ id: HERE, name: 'Personal', own: true, current: true }],
      });
    TestBed.configureTestingModule({
      providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
    });
    localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
    await TestBed.inject(SessionService).start();
    await TestBed.inject(OrganizationsService).load();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it('opens a tapped thread of this tab’s organization here, without loading the page again', () => {
    const navigated = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    expect(ask(TestBed.inject(TapsService), `/t/${THREAD}`, HERE)).toBe(true);
    expect(navigated).toHaveBeenCalledWith(`/t/${THREAD}`);
  });

  it('leaves one of another organization, or any other message, to the worker', () => {
    const navigated = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const taps = TestBed.inject(TapsService);
    expect(ask(taps, `/t/${THREAD}`, ELSEWHERE)).toBe(false);
    expect(ask(taps, '/settings', null)).toBe(false);
    expect(navigated).not.toHaveBeenCalled();
  });
});
