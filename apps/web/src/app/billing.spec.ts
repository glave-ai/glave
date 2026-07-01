import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import {
  HttpTestingController,
  provideHttpClientTesting,
} from '@angular/common/http/testing';
import { BillingPanel, BillingService, type BillingStatus } from './billing';

// The panel's `ngOnInit` runs an async load chain Angular doesn't await, so drain
// pending microtasks and re-render until the signals have settled.
async function settle(fixture: ComponentFixture<unknown>): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await fixture.whenStable();
    await Promise.resolve();
    fixture.detectChanges();
  }
}

const OWNER_STATUS: BillingStatus = {
  account_id: 'acct_1',
  role: 'owner',
  balance_usd: 12.5,
  org_limit_usd: 5,
  member_limit_usd: null,
};

describe('BillingService', () => {
  let service: BillingService;
  let httpMock: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    service = TestBed.inject(BillingService);
    httpMock = TestBed.inject(HttpTestingController);
  });

  afterEach(() => httpMock.verify());

  it('fetches status with credentials', async () => {
    const promise = service.status();
    const req = httpMock.expectOne('/billing/status');
    expect(req.request.method).toBe('GET');
    expect(req.request.withCredentials).toBe(true);
    req.flush(OWNER_STATUS);
    expect(await promise).toEqual(OWNER_STATUS);
  });

  it('sets an org limit', async () => {
    const promise = service.setLimit(20, 'month');
    const req = httpMock.expectOne('/billing/limit');
    expect(req.request.method).toBe('POST');
    expect(req.request.body).toEqual({
      scope: 'org',
      amount_usd: 20,
      period: 'month',
    });
    req.flush(null, { status: 204, statusText: 'No Content' });
    await promise;
  });

  it('clears the limit with a null amount', async () => {
    const promise = service.setLimit(null, 'day');
    const req = httpMock.expectOne('/billing/limit');
    expect(req.request.body).toEqual({
      scope: 'org',
      amount_usd: null,
      period: 'day',
    });
    req.flush(null, { status: 204, statusText: 'No Content' });
    await promise;
  });

  it('returns a redirect url on checkout success', async () => {
    const promise = service.checkout(50);
    const req = httpMock.expectOne('/billing/checkout');
    expect(req.request.body).toEqual({ amount_usd: 50 });
    req.flush({ url: 'https://pay.example/session' });
    expect(await promise).toEqual({
      kind: 'redirect',
      url: 'https://pay.example/session',
    });
  });

  it('surfaces a 503 as a not_configured outcome, not an error', async () => {
    const promise = service.checkout(25);
    httpMock
      .expectOne('/billing/checkout')
      .flush(null, { status: 503, statusText: 'Service Unavailable' });
    expect(await promise).toEqual({ kind: 'not_configured' });
  });
});

describe('BillingPanel', () => {
  let httpMock: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [BillingPanel],
      providers: [provideHttpClient(), provideHttpClientTesting()],
    }).compileComponents();
    httpMock = TestBed.inject(HttpTestingController);
  });

  afterEach(() => httpMock.verify());

  it('loads status on init and shows the balance', async () => {
    const fixture = TestBed.createComponent(BillingPanel);
    fixture.detectChanges();
    httpMock.expectOne('/billing/status').flush(OWNER_STATUS);
    // Let ngOnInit's async chain settle the loading/status signals, then render.
    await settle(fixture);
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('.balance')?.textContent).toContain('$12.50');
    expect(compiled.querySelector('.limit-current')?.textContent).toContain('$5.00');
  });

  it('shows "No limit" and hides owner controls for a member', async () => {
    const fixture = TestBed.createComponent(BillingPanel);
    fixture.detectChanges();
    httpMock.expectOne('/billing/status').flush({
      ...OWNER_STATUS,
      role: 'member',
      org_limit_usd: null,
    } satisfies BillingStatus);
    await settle(fixture);
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('.limit-current')?.textContent).toContain('No limit');
    expect(compiled.querySelector('.limit-form')).toBeNull();
  });
});
